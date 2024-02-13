use chrono::{DateTime, Utc};

#[derive(serde::Serialize)]
#[serde(tag = "type")]
pub enum WebhookMessage {
    #[serde(rename = "mo_message")]
    MOMessage(MOMessage),
    #[serde(rename = "mt_message_status")]
    MTMessageStatus(MTMessageStatus),
}

#[derive(serde::Serialize)]
pub struct MOMessage {
    pub id: uuid::Uuid,
    pub header: MOHeader,
    pub location_information: Option<MOLocationInformation>,
    pub payload: Option<String>,
}

#[derive(serde::Serialize)]
pub struct MOHeader {
    pub imei: String,
    pub cdr_reference: u32,
    pub session_status: SessionStatus,
    pub mo_msn: u16,
    pub mt_msn: u16,
    pub time_of_session: DateTime<Utc>
}

#[derive(serde::Serialize)]
pub struct MOLocationInformation {
    pub latitude: f32,
    pub longitude: f32,
    pub cep_radius: u32,
}

#[derive(serde::Serialize)]
pub enum SessionStatus {
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "too_large")]
    TooLarge,
    #[serde(rename = "unacceptable_location")]
    UnacceptableLocation,
}

#[derive(serde::Serialize)]
pub struct MTMessageStatus {
    pub id: uuid::Uuid,
    pub status: MessageStatus
}

#[derive(serde::Serialize)]
pub enum MessageStatus {
    #[serde(rename = "delivered")]
    Delivered,
    #[serde(rename = "invalid_imei")]
    InvalidIMEI,
    #[serde(rename = "payload_size_exceeded")]
    PayloadSizeExceeded,
    #[serde(rename = "message_queue_full")]
    MessageQueueFull,
    #[serde(rename = "resources_unavailable")]
    ResourcesUnavailable,
}

#[derive(serde::Deserialize)]
pub struct MTMessage {
    pub imei: String,
    pub payload: String,
    #[serde(default)]
    pub priority: Option<u8>,
}