use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::{AsyncConnection, RunQueryDsl};
use diesel_async::scoped_futures::ScopedFutureExt;
use hmac::Mac;

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

pub async fn run_worker(db_pool: crate::DBPool) {
    let client = std::sync::Arc::new(reqwest::ClientBuilder::new()
        .user_agent(format!("Kosmos {}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build().unwrap()
    );

    let mut db_conn = match db_pool.get().await {
        Ok(c) => c,
        Err(err) => {
            error!("Failed to get DB connection: {}", err);
            return;
        }
    };

    info!("Kosmos worker running");

    loop {
        let db_pool = db_pool.clone();
        let client = client.clone();
        if let Err(err) = db_conn.transaction(|db_conn| async move {
            let messages = crate::schema::mo_messages::dsl::mo_messages.filter(
                crate::schema::mo_messages::dsl::processing_status.eq(crate::models::ProcessingStatus::Received)
            ).get_results::<crate::models::MOMessage>(db_conn).await?;

            let ids = messages.iter().map(|m| m.id).collect::<Vec<_>>();
            diesel::update(crate::schema::mo_messages::dsl::mo_messages)
                .filter(crate::schema::mo_messages::dsl::id.eq_any(&ids))
                .set(crate::schema::mo_messages::dsl::processing_status.eq(crate::models::ProcessingStatus::Processing))
                .execute(db_conn).await?;

            for m in messages {
                let new_db_conn = db_pool.get().await.map_err(|e| {
                    diesel::result::Error::QueryBuilderError(Box::new(e))
                })?;
                tokio::spawn(process_message(m, new_db_conn, client.clone()));
            }

            Ok::<(), diesel::result::Error>(())
        }.scope_boxed()).await {
            error!("Failed to query database for messages: {}", err);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await
    }
}

async fn process_message(message: crate::models::MOMessage, mut db_conn: crate::DBConn, client: std::sync::Arc<reqwest::Client>) {
    let id = message.id;
    if !matches!(message.session_status, crate::models::SessionStatus::Successful |
        crate::models::SessionStatus::SuccessfulTooLarge | crate::models::SessionStatus::SuccessfulUnacceptableLocation) {
        if let Err(err) = diesel::update(crate::schema::mo_messages::dsl::mo_messages)
            .filter(crate::schema::mo_messages::dsl::id.eq(id))
            .set(crate::schema::mo_messages::dsl::processing_status.eq(crate::models::ProcessingStatus::Done))
            .execute(&mut db_conn).await {
            error!("Failed to update message: {}", err);
            return;
        }
    }

    let new_status = if _process_message(message, &mut db_conn, client).await {
        crate::models::ProcessingStatus::Done
    } else {
        crate::models::ProcessingStatus::Received
    };

    if let Err(err) = diesel::update(crate::schema::mo_messages::dsl::mo_messages)
        .filter(crate::schema::mo_messages::dsl::id.eq(id))
        .set(crate::schema::mo_messages::dsl::processing_status.eq(new_status))
        .execute(&mut db_conn).await {
        error!("Failed to update message: {}", err);
        return;
    }
}

async fn _process_message(message: crate::models::MOMessage, db_conn: &mut crate::DBConn, client: std::sync::Arc<reqwest::Client>) -> bool {
    let (target, _) = match crate::schema::targets::dsl::targets
        .inner_join(crate::schema::devices::dsl::devices)
        .filter(crate::schema::devices::dsl::imei.eq(&message.imei))
        .select((crate::models::Target::as_select(), crate::models::Device::as_select()))
        .get_result::<(crate::models::Target, crate::models::Device)>(db_conn).await.optional() {
        Ok(Some(d)) => d,
        Ok(None) => {
            return true;
        }
        Err(err) => {
            error!("Failed to get target: {}", err);
            return false;
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
        payload: message.data.map(|d| hex::encode(d)),
    };

    let mut mac = HmacSha256::new_from_slice(&target.hmac_key).unwrap();
    let message_to_send_bytes = serde_json::to_vec(&message_to_send).unwrap();
    mac.update(&message_to_send_bytes);
    let mac_result = mac.finalize().into_bytes();

    let res = match client.post(&target.endpoint)
        .header("Kosmos-MAC", hex::encode(mac_result))
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
        return false;
    }

    true
}