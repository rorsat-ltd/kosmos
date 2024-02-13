use base64::prelude::*;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use hmac::Mac;
use rocket::data::ToByteUnit;

struct Auth {
    id: uuid::Uuid,
    signature: Vec<u8>
}

#[rocket::async_trait]
impl<'r> rocket::request::FromRequest<'r> for Auth {
    type Error = ();

    async fn from_request(request: &'r rocket::request::Request<'_>) -> rocket::request::Outcome<Self, Self::Error> {
        let id = request.headers().get_one("Kosmos-Target-ID");
        let token = request.headers().get_one("Kosmos-MAC");
        match (id, token) {
            (Some(id), Some(token)) => match BASE64_STANDARD.decode(token) {
                Ok(t) => match uuid::Uuid::parse_str(id) {
                    Ok(i) => rocket::request::Outcome::Success(Auth {
                        id: i,
                        signature: t
                    }),
                    Err(_) => rocket::request::Outcome::Error((rocket::http::Status::Unauthorized, ()))
                },
                Err(_) => rocket::request::Outcome::Error((rocket::http::Status::Unauthorized, ()))
            },
            _ => rocket::request::Outcome::Error((rocket::http::Status::Unauthorized, ()))
        }
    }
}

#[rocket::get("/submit_mt")]
fn submit_mt_get() -> rocket::http::Status {
    rocket::http::Status::MethodNotAllowed
}

#[rocket::post("/submit_mt", data = "<data>", format = "application/json")]
async fn submit_mt(
    db: &rocket::State<crate::DBPool>, celery_app: &rocket::State<std::sync::Arc<celery::Celery>>,
    auth: Auth, data: rocket::data::Data<'_>
) -> Result<String, rocket::http::Status> {
    let mut db_conn = match db.get().await {
        Ok(c) => c,
        Err(err) => {
            error!("Failed to get DB connection: {}", err);
            return Err(rocket::http::Status::InternalServerError);
        }
    };

    let target = match crate::schema::targets::dsl::targets.filter(
        crate::schema::targets::dsl::id.eq(&auth.id)
    ).get_result::<crate::models::Target>(&mut db_conn).await
        .optional() {
        Ok(Some(t)) => t,
        Ok(None) => {
            return Err(rocket::http::Status::Unauthorized);
        }
        Err(err) => {
            error!("Failed to get target: {}", err);
            return Err(rocket::http::Status::InternalServerError);
        }
    };

    let mut mac = crate::HmacSha256::new_from_slice(&target.hmac_key).unwrap();

    let body = data.open(4.kibibytes()).into_bytes().await
        .map_err(|_| rocket::http::Status::InternalServerError)?;
    if !body.is_complete() {
        return Err(rocket::http::Status::PayloadTooLarge);
    }
    mac.update(&body);
    let mac_result = mac.finalize().into_bytes();

    if !constant_time_eq::constant_time_eq(&mac_result, &auth.signature) {
        return Err(rocket::http::Status::Unauthorized);
    }

    let request: crate::types::MTMessage = serde_json::from_slice(&body)
        .map_err(|_| rocket::http::Status::BadRequest)?;

    if request.imei.len() != 15 {
        return Err(rocket::http::Status::Unauthorized);
    }
    if !request.imei.chars().all(|c| c.is_ascii_digit()) {
        return Err(rocket::http::Status::BadRequest);
    }

    let msg_data = BASE64_STANDARD.decode(request.payload)
        .map_err(|_| rocket::http::Status::BadRequest)?;

    let priority = if let Some(p) = request.priority {
        if p < 1 || p > 5 {
            return Err(rocket::http::Status::BadRequest);
        }
        p as i16
    } else {
        0
    };

    let mt_message = crate::models::MTMessage {
        id: uuid::Uuid::new_v4(),
        imei: request.imei,
        data: msg_data,
        priority,
        message_status: None,
        processing_status: crate::models::ProcessingStatus::Received,
        received: chrono::Utc::now().naive_utc(),
        target: target.id
    };

    if let Err(err) = diesel::insert_into(crate::schema::mt_messages::dsl::mt_messages)
        .values(&mt_message)
        .execute(&mut db_conn).await {
        error!("Failed to insert message: {}", err);
        return Err(rocket::http::Status::InternalServerError);
    }

    if let Err(err) = celery_app.send_task(crate::worker::deliver_mt::new(mt_message.id)).await {
        error!("Failed to send task: {}", err);
        return Err(rocket::http::Status::InternalServerError);
    }

    Ok(mt_message.id.to_string())
}

pub async fn run(listen_addr: std::net::SocketAddr, amqp_addr: String, db_pool: crate::DBPool) {
    let figment = rocket::Config::figment()
        .merge(("address", listen_addr.ip()))
        .merge(("port", listen_addr.port()))
        .merge(("ident", format!("Kosmos {}", env!("CARGO_PKG_VERSION"))));

    let celery_app = match celery::app!(
        broker = AMQP { amqp_addr },
        tasks = [],
        task_routes = [],
        acks_late = true,
    ).await {
        Ok(a) => a,
        Err(err) => {
            error!("Failed to setup celery: {}", err);
            return;
        }
    };

    rocket::custom(figment)
        .mount("/", rocket::routes![
            submit_mt,
            submit_mt_get,
        ])
        .manage(celery_app)
        .manage(db_pool)
        .launch().await.unwrap();
}