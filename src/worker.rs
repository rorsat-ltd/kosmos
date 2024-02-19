use celery::prelude::*;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use hmac::Mac;
use base64::prelude::*;


static HTTP_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
static DB_POOL: std::sync::OnceLock<crate::DBPool> = std::sync::OnceLock::new();
static CELERY_APP: std::sync::OnceLock<std::sync::Arc<celery::Celery>> = std::sync::OnceLock::new();

pub async fn run_worker(amqp_addr: String, db_pool: crate::DBPool) {
    let client = reqwest::ClientBuilder::new()
        .user_agent(format!("Kosmos {}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build().unwrap();

    let celery_app = match celery::app!(
        broker = AMQP { amqp_addr },
        tasks = [process_message, deliver_mt],
        task_routes = [],
        acks_late = true,
    ).await {
        Ok(a) => a,
        Err(err) => {
            error!("Failed to setup celery: {}", err);
            return;
        }
    };

    let _ = HTTP_CLIENT.set(client);
    let _ = DB_POOL.set(db_pool);
    let _ = CELERY_APP.set(celery_app.clone());

    info!("Kosmos worker running");

    celery_app.consume().await.unwrap();
}

async fn set_mo_message_status(message_id: uuid::Uuid, status: crate::models::ProcessingStatus, db_conn: &mut crate::DBConn) -> TaskResult<()> {
    diesel::update(crate::schema::mo_messages::dsl::mo_messages)
        .filter(crate::schema::mo_messages::dsl::id.eq(message_id))
        .set(crate::schema::mo_messages::dsl::processing_status.eq(status))
        .execute(db_conn).await
        .with_expected_err(|| "Failed to update MO message")?;
    Ok(())
}

async fn set_mt_processing_status(message_id: uuid::Uuid, status: crate::models::ProcessingStatus, db_conn: &mut crate::DBConn) -> TaskResult<()> {
    diesel::update(crate::schema::mt_messages::dsl::mt_messages)
        .filter(crate::schema::mt_messages::dsl::id.eq(message_id))
        .set(crate::schema::mt_messages::dsl::processing_status.eq(status))
        .execute(db_conn).await
        .with_expected_err(|| "Failed to update MT message processing status")?;
    Ok(())
}

async fn set_mt_message_status(message_id: uuid::Uuid, status: crate::models::MessageStatus, db_conn: &mut crate::DBConn) -> TaskResult<()> {
    diesel::update(crate::schema::mt_messages::dsl::mt_messages)
        .filter(crate::schema::mt_messages::dsl::id.eq(message_id))
        .set(crate::schema::mt_messages::dsl::message_status.eq(status))
        .execute(db_conn).await
        .with_expected_err(|| "Failed to update MT message status")?;
    Ok(())
}

async fn send_webhook<D: serde::ser::Serialize>(target: &crate::models::Target, data: &D) -> bool {
    let mut mac = crate::HmacSha256::new_from_slice(&target.hmac_key).unwrap();
    let message_to_send_bytes = serde_json::to_vec(data).unwrap();
    mac.update(&message_to_send_bytes);
    let mac_result = mac.finalize().into_bytes();

    let res = match HTTP_CLIENT.get().unwrap().post(&target.endpoint)
        .header("Kosmos-MAC", BASE64_STANDARD.encode(mac_result))
        .header("Content-Type", "application/json")
        .body(message_to_send_bytes)
        .send().await {
        Ok(d) => d,
        Err(err) => {
            warn!("Failed to send to target {}: {}", target.endpoint, err);

            return false;
        }
    };

    if !res.status().is_success() {
        warn!("Received error response from target {}", target.endpoint);
        false
    } else {
        true
    }
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
        set_mo_message_status(message_id, crate::models::ProcessingStatus::Done, &mut db_conn).await?;
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
            set_mo_message_status(message_id, crate::models::ProcessingStatus::Done, &mut db_conn).await?;
            return Ok(());
        }
    };

    let message_to_send = crate::types::WebhookMessage::MOMessage(crate::types::MOMessage {
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
    });

    let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);
    if !send_webhook(&target, &message_to_send).await {
        return if cutoff > message.received.and_utc() {
            set_mo_message_status(message_id, crate::models::ProcessingStatus::Failed, &mut db_conn).await?;
            Ok(())
        } else {
            task.retry_with_countdown(60)
        }
    }

    set_mo_message_status(message_id, crate::models::ProcessingStatus::Done, &mut db_conn).await?;

    Ok(())
}

#[celery::task(bind = true)]
pub async fn deliver_mt(task: &Self, message_id: uuid::Uuid) -> TaskResult<()> {
    let mut db_conn = DB_POOL.get().unwrap().get().await
        .with_expected_err(|| "Failed to get DB connection")?;

    let message = crate::schema::mt_messages::dsl::mt_messages.filter(
        crate::schema::mt_messages::dsl::id.eq(message_id)
    ).get_result::<crate::models::MTMessage>(&mut db_conn).await
        .with_expected_err(|| "Failed to get message from DB")?;

    let mut socket = tokio::net::TcpStream::connect(crate::IRIDUM_MT_ADDR).await
        .with_expected_err(|| "Failed to connect to Iridium gateway")?;

    let client_message_id: u32 = rand::random();

    let header = crate::ie::MTHeader {
        client_message_id,
        imei: message.imei,
        flush_mt_queue: false,
        send_ring_alert: false,
        update_ssd_location: false,
        high_priority_message: message.priority != 0,
        assign_mtmsn: false,
    };

    let payload = crate::ie::MTPayload {
        data: message.data
    };

    let mut elements = vec![header.to_element(), payload.to_element()];

    if message.priority != 0 {
        elements.push(crate::ie::MTPriority {
            level: message.priority as u16,
        }.to_element());
    }

    let protocol_message = crate::ie::ProtocolMessage {
        elements
    };

    trace!("Sending message: {:02x?}", protocol_message);

    protocol_message.write(&mut socket).await
        .with_expected_err(|| "Failed to write protocol message to socket")?;

    let response_protocol_message = crate::ie::ProtocolMessage::read(&mut socket).await
        .with_expected_err(|| "Failed to read response protocol message from socket")?;
    let response_message = crate::ie::ResponseMessage::from_pm(response_protocol_message)
        .with_unexpected_err(|| "Failed to decode response message")?;

    trace!("Got response: {:02x?}", response_message);

    if response_message.confirmation.client_message_id != header.client_message_id ||
        response_message.confirmation.imei != header.imei {
        warn!("Response not for sent message");
        return task.retry_with_countdown(60);
    }

    let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);

    match response_message.confirmation.message_status {
        crate::ie::MessageStatus::Successful(_) => {
            set_mt_processing_status(message_id, crate::models::ProcessingStatus::Done, &mut db_conn).await?;
            set_mt_message_status(message_id, crate::models::MessageStatus::Delivered, &mut db_conn).await?;
        }
        crate::ie::MessageStatus::SuccessfulNoPayload => {
            warn!("Expected payload to be acknowledged");
            return task.retry_with_countdown(60);
        }
        crate::ie::MessageStatus::UnknownIMEI => {
            set_mt_processing_status(message_id, crate::models::ProcessingStatus::Done, &mut db_conn).await?;
            set_mt_message_status(message_id, crate::models::MessageStatus::InvalidImei, &mut db_conn).await?;
        }
        crate::ie::MessageStatus::TooLarge => {
            set_mt_processing_status(message_id, crate::models::ProcessingStatus::Done, &mut db_conn).await?;
            set_mt_message_status(message_id, crate::models::MessageStatus::PayloadSizeExceeded, &mut db_conn).await?;
        }
        crate::ie::MessageStatus::QueueFull => {
            if cutoff > message.received.and_utc() {
                set_mt_processing_status(message_id, crate::models::ProcessingStatus::Failed, &mut db_conn).await?;
                set_mt_message_status(message_id, crate::models::MessageStatus::MessageQueueFull, &mut db_conn).await?;
            } else {
                return task.retry_with_countdown(60);
            }
        }
        crate::ie::MessageStatus::ResourcesUnavailable => {
            warn!("Failed to send MT message, Iridium resources unavailable");
            if cutoff > message.received.and_utc() {
                set_mt_processing_status(message_id, crate::models::ProcessingStatus::Failed, &mut db_conn).await?;
                set_mt_message_status(message_id, crate::models::MessageStatus::ResourcesUnavailable, &mut db_conn).await?;
            } else {
                return task.retry_with_countdown(60);
            }
        }
        o => {
            set_mt_processing_status(message_id, crate::models::ProcessingStatus::Failed, &mut db_conn).await?;
            return Err(TaskError::UnexpectedError(format!("unexpected response status: {:?}", o)));
        }
    }

    if let Err(err) = CELERY_APP.get().unwrap().send_task(send_mt_status::new(message.id)).await {
        error!("Failed to send MT status task: {}", err);
    }

    Ok(())
}

#[celery::task(bind = true)]
pub async fn send_mt_status(task: &Self, message_id: uuid::Uuid) -> TaskResult<()> {
    let mut db_conn = DB_POOL.get().unwrap().get().await
        .with_expected_err(|| "Failed to get DB connection")?;

    let message = crate::schema::mt_messages::dsl::mt_messages.filter(
        crate::schema::mt_messages::dsl::id.eq(message_id)
    ).get_result::<crate::models::MTMessage>(&mut db_conn).await
        .with_expected_err(|| "Failed to get message from DB")?;
    let target = crate::schema::targets::dsl::targets.filter(
        crate::schema::targets::dsl::id.eq(message.target)
    ).get_result::<crate::models::Target>(&mut db_conn).await
        .with_expected_err(|| "Failed to get target from DB")?;

    let message_status = match message.message_status {
        Some(s) => s,
        None => {
            return Ok(())
        }
    };

    let message_to_send = crate::types::WebhookMessage::MTMessageStatus(crate::types::MTMessageStatus {
        id: message.id,
        status: match message_status {
            crate::models::MessageStatus::Delivered => crate::types::MessageStatus::Delivered,
            crate::models::MessageStatus::InvalidImei => crate::types::MessageStatus::InvalidIMEI,
            crate::models::MessageStatus::PayloadSizeExceeded => crate::types::MessageStatus::PayloadSizeExceeded,
            crate::models::MessageStatus::MessageQueueFull => crate::types::MessageStatus::MessageQueueFull,
            crate::models::MessageStatus::ResourcesUnavailable => crate::types::MessageStatus::ResourcesUnavailable,
        },
    });

    if !send_webhook(&target, &message_to_send).await {
        return task.retry_with_countdown(60);
    }

    Ok(())
}