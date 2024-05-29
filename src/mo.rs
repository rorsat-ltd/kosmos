use diesel_async::RunQueryDsl;
use rocket::form::validate::Contains;


pub async fn receive_mo(
    listen_address: std::net::SocketAddr,
    amqp_addr: String,
    nat64_prefix: Option<ipnetwork::Ipv6Network>,
    source_ips: Vec<std::net::IpAddr>,
    db_pool: crate::DBPool,
) {
    let celery_app = match celery::app!(
        broker = AMQP { amqp_addr },
        tasks = [crate::worker::process_message],
        task_routes = [],
    ).await {
        Ok(a) => a,
        Err(err) => {
            error!("Failed to setup celery: {}", err);
            return;
        }
    };

    let listener = match tokio::net::TcpListener::bind(listen_address).await {
        Ok(l) => l,
        Err(err) => {
            error!("Failed to open TCP socket: {}", err);
            return;
        }
    };

    info!("Listening for SBD on {}", listener.local_addr().unwrap());

    loop {
        let (socket, peer_address) = match listener.accept().await {
            Ok(l) => l,
            Err(err) => {
                error!("Failed to receive TCP connection: {}", err);
                return;
            }
        };

        let real_ip = match (peer_address.ip(), nat64_prefix) {
            (std::net::IpAddr::V4(a), _) => std::net::IpAddr::V4(a),
            (std::net::IpAddr::V6(a), None) => std::net::IpAddr::V6(a),
            (std::net::IpAddr::V6(a), Some(n)) => {
                if n.contains(a) {
                    let [_, _, _, _, _, _, ab, cd] = a.segments();
                    let [a, b] = ab.to_be_bytes();
                    let [c, d] = cd.to_be_bytes();
                    std::net::IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d))
                } else {
                    std::net::IpAddr::V6(a)
                }
            },
        };
        info!("Connection received from {} ({})", std::net::SocketAddr::new(real_ip, peer_address.port()), peer_address);

        if (source_ips.is_empty() && real_ip != crate::IRIDIUM_SOURCE_IP) || !source_ips.contains(real_ip) {
            warn!("Connection not from Iridium, dropping");
            continue;
        }

        tokio::spawn(process_socket(socket, db_pool.clone(), celery_app.clone()));
    }
}

async fn process_socket(
    mut socket: tokio::net::TcpStream,
    db_pool: crate::DBPool,
    celery_app: std::sync::Arc<celery::Celery>
) {
    let status = _process_socket(&mut socket, db_pool, celery_app).await;

    let confirmation = crate::ie::MOConfirmation {
        status,
    };
    let confirmation_elm = confirmation.to_element();
    let confirmation_pm = crate::ie::ProtocolMessage {
        elements: vec![confirmation_elm]
    };
    if let Err(err) = confirmation_pm.write(&mut socket).await {
        warn!("Failed to send response: {:?}", err);
    }
}

async fn _process_socket(
    socket: &mut tokio::net::TcpStream,
    db_pool: crate::DBPool,
    celery_app: std::sync::Arc<celery::Celery>
) -> bool {
    let protocol_message = match crate::ie::ProtocolMessage::read(socket).await {
        Ok(m) => m,
        Err(err) => {
            warn!("Failed to decode protocol message: {:?}", err);
            return false;
        }
    };

    let message = match crate::ie::Message::from_pm(protocol_message) {
        Ok(m) => m,
        Err(err) => {
            warn!("Failed to decode message: {:?}", err);
            return false;
        }
    };
    debug!("Received message: {:#?}", message);

    let message_to_save = crate::models::MOMessage {
        id: uuid::Uuid::new_v4(),
        cdr_reference: message.header.cdr_reference as i32,
        imei: message.header.imei,
        session_status: message.header.session_status.into(),
        mo_msn: message.header.momsn as i16,
        mt_msn: message.header.mtmsn as i16,
        time_of_session: message.header.time_of_session.naive_utc(),
        latitude: message.location_information.as_ref().map(|l| l.latitude),
        longitude: message.location_information.as_ref().map(|l| l.longitude),
        cep_radius: message.location_information.as_ref().map(|l| l.cep_radius as i32),
        data: message.payload,
        processing_status: crate::models::ProcessingStatus::Received,
        received: chrono::Utc::now().naive_utc(),
    };

    let mut db_conn = match db_pool.get().await {
        Ok(c) => c,
        Err(err) => {
            error!("Failed to get DB connection: {}", err);
            return false;
        }
    };
    if let Err(err) = diesel::insert_into(crate::schema::mo_messages::dsl::mo_messages)
        .values(&message_to_save)
        .execute(&mut db_conn).await {
        error!("Failed to insert message: {}", err);
        return false;
    }

    if let Err(err) = celery_app.send_task(crate::worker::process_message::new(message_to_save.id)).await {
        error!("Failed to send task: {}", err);
        return false;
    }

    true
}