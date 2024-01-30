use chrono::{DateTime, Utc};

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