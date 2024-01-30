use celery::prelude::*;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use hmac::Mac;
use base64::prelude::*;

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

static HTTP_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
static DB_POOL: std::sync::OnceLock<crate::DBPool> = std::sync::OnceLock::new();

pub async fn run_worker(amqp_addr: String, db_pool: crate::DBPool) {
    let client = reqwest::ClientBuilder::new()
        .user_agent(format!("Kosmos {}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build().unwrap();

    let celery_app = match celery::app!(
        broker = AMQP { amqp_addr },
        tasks = [process_message],
        task_routes = [],
        acks_late = true,
    ).await {
        Ok(a) => a,
        Err(err) => {
            error!("Failed to setup celery: {}", err);
            return;
        }
    };

    HTTP_CLIENT.set(client).unwrap();
    DB_POOL.set(db_pool).unwrap();

    info!("Kosmos worker running");

    celery_app.consume().await.unwrap();
}

async fn set_message_status(message_id: uuid::Uuid, status: crate::models::ProcessingStatus, db_conn: &mut crate::DBConn) -> TaskResult<()> {
    diesel::update(crate::schema::mo_messages::dsl::mo_messages)
        .filter(crate::schema::mo_messages::dsl::id.eq(message_id))
        .set(crate::schema::mo_messages::dsl::processing_status.eq(status))
        .execute(db_conn).await
        .with_expected_err(|| "Failed to update message")?;
    Ok(())
}

#[celery::task(bind = true)]
pub async fn process_message(task: &Self, message_id: uuid::Uuid) -> TaskResult<()> {
    let mut db_conn = DB_POOL.get().unwrap().get().await
        .with_expected_err(|| "Failed to get DB connection")?;

    let message = crate::schema::mo_messages::dsl::mo_messages.filter(
        crate::schema::mo_messages::dsl::id.eq(message_id)
    ).get_result::<crate::models::MOMessage>(&mut db_conn).await
        .with_expected_err(|| "Failed to get message from DB")?;

    if !matches!(message.session_status, crate::models::SessionStatus::Successful |
        crate::models::SessionStatus::SuccessfulTooLarge | crate::models::SessionStatus::SuccessfulUnacceptableLocation) {
        set_message_status(message_id, crate::models::ProcessingStatus::Done, &mut db_conn).await?;
        return Ok(());
    }

    let (target, _) = match crate::schema::targets::dsl::targets
        .inner_join(crate::schema::devices::dsl::devices)
        .filter(crate::schema::devices::dsl::imei.eq(&message.imei))
        .select((crate::models::Target::as_select(), crate::models::Device::as_select()))
        .get_result::<(crate::models::Target, crate::models::Device)>(&mut db_conn).await.optional()
        .with_expected_err(|| "Failed to get target")? {
        Some(d) => d,
        None => {
            set_message_status(message_id, crate::models::ProcessingStatus::Done, &mut db_conn).await?;
            return Ok(());
        }
    };

    let message_to_send = crate::types::MOMessage {
        id: message.id,
        header: crate::types::MOHeader {
            imei: message.imei,
            cdr_reference: message.cdr_reference as u32,
            session_status: match message.session_status {
                crate::models::SessionStatus::Successful => crate::types::SessionStatus::Normal,
                crate::models::SessionStatus::SuccessfulTooLarge => crate::types::SessionStatus::TooLarge,
                crate::models::SessionStatus::SuccessfulUnacceptableLocation => crate::types::SessionStatus::UnacceptableLocation,
                _ => unreachable!()
            },
            mo_msn: message.mo_msn as u16,
            mt_msn: message.mt_msn as u16,
            time_of_session: message.time_of_session.and_utc(),
        },
        location_information: match (message.longitude, message.latitude, message.cep_radius) {
            (Some(longitude), Some(latitude), Some(cep_radius)) => Some(crate::types::MOLocationInformation {
                longitude,
                latitude,
                cep_radius: cep_radius as u32
            }),
            _ => None
        },
        payload: message.data.map(|d| BASE64_STANDARD.encode(d)),
    };

    let mut mac = HmacSha256::new_from_slice(&target.hmac_key).unwrap();
    let message_to_send_bytes = serde_json::to_vec(&message_to_send).unwrap();
    mac.update(&message_to_send_bytes);
    let mac_result = mac.finalize().into_bytes();

    let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);

    let res = match HTTP_CLIENT.get().unwrap().post(&target.endpoint)
        .header("Kosmos-MAC", BASE64_STANDARD.encode(mac_result))
        .header("Content-Type", "application/json")
        .body(message_to_send_bytes)
        .send().await {
        Ok(d) => d,
        Err(err) => {
            warn!("Failed to send to target {}: {}", target.endpoint, err);

            return if cutoff > message.received.and_utc() {
                set_message_status(message_id, crate::models::ProcessingStatus::Failed, &mut db_conn).await?;
                Ok(())
            } else {
                task.retry_with_countdown(60)
            }
        }
    };

    if !res.status().is_success() {
        warn!("Received error response from target {}", target.endpoint);

        return if cutoff > message.received.and_utc() {
            set_message_status(message_id, crate::models::ProcessingStatus::Failed, &mut db_conn).await?;
            Ok(())
        } else {
            task.retry_with_countdown(60)
        }
    }

    set_message_status(message_id, crate::models::ProcessingStatus::Done, &mut db_conn).await?;

    Ok(())
}