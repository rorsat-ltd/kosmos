#[derive(Debug, diesel_derive_enum::DbEnum)]
#[ExistingTypePath = "crate::schema::sql_types::SessionStatus"]
pub enum SessionStatus {
    Successful,
    SuccessfulTooLarge,
    SuccessfulUnacceptableLocation,
    Timeout,
    TooLarge,
    RfLinkLost,
    ProtocolAnomaly,
    ImeiBlocked
}

impl From<crate::ie::SessionStatus> for SessionStatus {
    fn from(value: crate::ie::SessionStatus) -> Self {
        match value {
            crate::ie::SessionStatus::Successful => Self::Successful,
            crate::ie::SessionStatus::SuccessfulTooLarge => Self::SuccessfulTooLarge,
            crate::ie::SessionStatus::SuccessfulUnacceptableLocation => Self::SuccessfulUnacceptableLocation,
            crate::ie::SessionStatus::Timeout => Self::Timeout,
            crate::ie::SessionStatus::TooLarge => Self::TooLarge,
            crate::ie::SessionStatus::RFLinkLost => Self::RfLinkLost,
            crate::ie::SessionStatus::ProtocolAnomaly => Self::ProtocolAnomaly,
            crate::ie::SessionStatus::IMEIBlocked => Self::ImeiBlocked,
        }
    }
}

#[derive(Debug, diesel_derive_enum::DbEnum)]
#[ExistingTypePath = "crate::schema::sql_types::MessageStatus"]
pub enum MessageStatus {
    Delivered,
    InvalidImei,
    PayloadSizeExceeded,
    MessageQueueFull,
    ResourcesUnavailable,
}

#[derive(Debug, diesel_derive_enum::DbEnum, PartialEq)]
#[ExistingTypePath = "crate::schema::sql_types::ProcessingStatus"]
pub enum ProcessingStatus {
    Received,
    Done,
    Failed,
}

#[derive(diesel::Queryable, diesel::Selectable, diesel::Insertable)]
#[diesel(table_name = crate::schema::mo_messages)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct MOMessage {
    pub id: uuid::Uuid,
    pub cdr_reference: i32,
    pub imei: String,
    pub session_status: SessionStatus,
    pub mo_msn: i16,
    pub mt_msn: i16,
    pub time_of_session: chrono::NaiveDateTime,
    pub latitude: Option<f32>,
    pub longitude: Option<f32>,
    pub cep_radius: Option<i32>,
    pub data: Option<Vec<u8>>,
    pub processing_status: ProcessingStatus,
    pub received: chrono::NaiveDateTime,
}

#[derive(diesel::Queryable, diesel::Selectable, diesel::Insertable)]
#[diesel(table_name = crate::schema::mt_messages)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct MTMessage {
    pub id: uuid::Uuid,
    pub imei: String,
    pub priority: i16,
    pub data: Vec<u8>,
    pub message_status: Option<MessageStatus>,
    pub processing_status: ProcessingStatus,
    pub received: chrono::NaiveDateTime,
    pub target: uuid::Uuid,
}

#[derive(diesel::Queryable, diesel::Selectable, diesel::Insertable)]
#[diesel(table_name = crate::schema::targets)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Target {
    pub id: uuid::Uuid,
    pub hmac_key: Vec<u8>,
    pub endpoint: String,
}

#[derive(diesel::Queryable, diesel::Selectable, diesel::Insertable)]
#[diesel(table_name = crate::schema::devices)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Device {
    pub id: uuid::Uuid,
    pub imei: String,
    pub target: uuid::Uuid
}