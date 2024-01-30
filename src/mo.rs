use diesel_async::RunQueryDsl;
use tokio::io::AsyncWriteExt;

pub async fn receive_mt(listen_address: std::net::SocketAddr, db_pool: crate::DBPool) {
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
        info!("Connection received from {}", peer_address);
        tokio::spawn(process_socket(socket, db_pool.clone()));
    }
}

async fn process_socket(mut socket: tokio::net::TcpStream, db_pool: crate::DBPool){
    let message = match crate::ie::Message::read(&mut socket).await {
        Ok(m) => m,
        Err(err) => {
            warn!("Failed to decode message: {:?}", err);
            return;
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
        last_processed: None,
    };

    let mut db_conn = match db_pool.get().await {
        Ok(c) => c,
        Err(err) => {
            error!("Failed to get DB connection: {}", err);
            return;
        }
    };
    if let Err(err) = diesel::insert_into(crate::schema::mo_messages::dsl::mo_messages)
        .values(&message_to_save)
        .execute(&mut db_conn).await {
        error!("Failed to insert message: {}", err);
        return;
    }

    let confirmation = crate::ie::Confirmation {
        status: true
    }.encode();
    let mut resp = vec![0x01, confirmation.len() as u8];
    resp.extend(confirmation.into_iter());
    if let Err(err) = socket.write_all(&resp).await {
        warn!("Failed to send response: {:?}", err);
    }
}