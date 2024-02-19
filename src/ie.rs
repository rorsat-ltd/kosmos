use std::fmt::{Display, Formatter};
use chrono::prelude::*;

#[derive(Debug)]
pub enum Error {
    UnsupportedProtocolRevision,
    FormatError(String),
    Io(std::io::Error)
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedProtocolRevision => f.write_str("unsupported protocol revision"),
            Self::FormatError(e) => f.write_fmt(format_args!("format error: {}", e)),
            Self::Io(e) => f.write_fmt(format_args!("I/O error: {}", e)),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Io(value)
    }
}

pub struct Message {
    pub header: MOHeader,
    pub payload: Option<Vec<u8>>,
    pub location_information: Option<LocationInformation>,
    pub extra: Vec<Element>,
}

impl Message {
    pub fn from_pm(pm: ProtocolMessage) -> Result<Self, Error> {
        let mut header = None;
        let mut payload = None;
        let mut location_information = None;
        let mut extra = vec![];

        for element in pm.elements {
            if element.id == 0x01 {
                if header.is_some() {
                    return Err(Error::FormatError("Duplicate header".to_string()));
                }
                header = Some(MOHeader::decode(&element.data)?);
            } else if element.id == 0x02 {
                if payload.is_some() {
                    return Err(Error::FormatError("Duplicate payload".to_string()));
                }
                payload = Some(element.data);
            } else if element.id == 0x03 {
                if location_information.is_some() {
                    return Err(Error::FormatError("Duplicate location information".to_string()));
                }
                location_information = Some(LocationInformation::decode(&element.data)?);
            } else {
                extra.push(element);
            }
        }

        if header.is_none() {
            return Err(Error::FormatError("Missing header".to_string()));
        }

        Ok(Self {
            header: header.unwrap(),
            payload,
            location_information,
            extra,
        })
    }
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "== MESSAGE ==")?;
        writeln!(f, "{:#?}", &self.header)?;
        if let Some(l) = &self.location_information {
            writeln!(f, "{:#?}", l)?;
        }
        for extra in &self.extra {
            writeln!(f, "{:#?}", extra)?;
        }
        if let Some(p) = &self.payload {
            writeln!(f, "{:02x?}", p)?;
        }
        writeln!(f, "== END MESSAGE ==")?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct ResponseMessage {
    pub confirmation: MTConfirmation,
    pub extra: Vec<Element>,
}

impl ResponseMessage {
    pub fn from_pm(pm: ProtocolMessage) -> Result<Self, Error> {
        let mut confirmation = None;
        let mut extra = vec![];

        for element in pm.elements {
            if element.id == 0x44 {
                if confirmation.is_some() {
                    return Err(Error::FormatError("Duplicate confirmation".to_string()));
                }
                confirmation = Some(MTConfirmation::decode(&element.data)?);
            } else {
                extra.push(element);
            }
        }

        if confirmation.is_none() {
            return Err(Error::FormatError("Missing confirmation".to_string()));
        }

        Ok(Self {
            confirmation: confirmation.unwrap(),
            extra,
        })
    }
}

#[derive(Debug)]
pub struct ProtocolMessage {
    pub elements: Vec<Element>
}

#[derive(Debug)]
pub struct Element {
    pub id: u8,
    pub data: Vec<u8>
}
#[derive(Debug)]
pub struct MOHeader {
    pub cdr_reference: u32,
    pub imei: String,
    pub session_status: SessionStatus,
    pub momsn: u16,
    pub mtmsn: u16,
    pub time_of_session: DateTime<Utc>
}

#[repr(u8)]
#[derive(Debug)]
pub enum SessionStatus {
    Successful = 0,
    SuccessfulTooLarge = 1,
    SuccessfulUnacceptableLocation = 2,
    Timeout = 10,
    TooLarge = 12,
    RFLinkLost = 13,
    ProtocolAnomaly = 14,
    IMEIBlocked = 15,
}

#[derive(Debug)]
pub struct LocationInformation {
    pub latitude: f32,
    pub longitude: f32,
    pub cep_radius: u32,
}

#[derive(Debug)]
pub struct MOConfirmation {
    pub status: bool
}

#[derive(Debug)]
pub struct MTHeader {
    pub client_message_id: u32,
    pub imei: String,
    pub flush_mt_queue: bool,
    pub send_ring_alert: bool,
    pub update_ssd_location: bool,
    pub high_priority_message: bool,
    pub assign_mtmsn: bool,
}

#[derive(Debug)]
pub struct MTConfirmation {
    pub client_message_id: u32,
    pub imei: String,
    pub auto_id_reference: u32,
    pub message_status: MessageStatus,
}

#[repr(i16)]
#[derive(Debug)]
pub enum MessageStatus {
    SuccessfulNoPayload = 0,
    Successful(u8),
    InvalidIMEI = -1,
    UnknownIMEI = -2,
    TooLarge = -3,
    PayloadExpected = -4,
    QueueFull = -5,
    ResourcesUnavailable = - 6,
    ProtocolViolation = -7,
    RingAlertsDisabled = -8,
    UnattachedIMEI = -9,
    IPBlocked = -10,
    MTMSNOutOfRange = -11,
}

#[derive(Debug)]
pub struct MTPayload {
    pub data: Vec<u8>
}

#[derive(Debug)]
pub struct MTPriority {
    pub level: u16,
}

impl ProtocolMessage {
    pub async fn read<R: tokio::io::AsyncRead + Unpin>(read: &mut R) -> Result<Self, Error> {
        use tokio::io::AsyncReadExt;

        let protocol_revision = read.read_u8().await?;
        if protocol_revision != 1 {
            return Err(Error::UnsupportedProtocolRevision)
        }

        let message_length = read.read_u16().await?;
        let mut message_data = vec![0u8; message_length as usize];
        read.read_exact(&mut message_data).await?;

        let mut elements = vec![];
        let mut message_data_cursor = std::io::Cursor::new(message_data);

        while message_data_cursor.position() < message_length as u64 {
            elements.push(Element::read(&mut message_data_cursor)?);
        }

        Ok(ProtocolMessage {
            elements
        })
    }

    pub async fn write<W: tokio::io::AsyncWrite + Unpin>(&self, write: &mut W) -> Result<(), Error> {
        use tokio::io::AsyncWriteExt;

        let mut message_data_cursor = std::io::Cursor::new(vec![]);
        for element in &self.elements {
            element.write(&mut message_data_cursor)?;
        }

        let message_data = message_data_cursor.into_inner();

        // Protocol revision
        write.write_u8(1).await?;
        // Data length
        write.write_u16(message_data.len() as u16).await?;
        // Data
        write.write_all(&message_data).await?;

        Ok(())
    }
}

impl Element {
    fn read<R: std::io::Read>(read: &mut R) -> Result<Self, Error> {
        use byteorder::{ReadBytesExt, BigEndian};

        let element_type = read.read_u8()?;
        let element_length = read.read_u16::<BigEndian>()?;
        let mut element_data = vec![0u8; element_length as usize];
        read.read_exact(&mut element_data)?;

        Ok(Element {
            id: element_type,
            data: element_data
        })
    }

    fn write<W: std::io::Write>(&self, write: &mut W) -> Result<(), Error> {
        use byteorder::{WriteBytesExt, BigEndian};

        write.write_u8(self.id)?;
        write.write_u16::<BigEndian>(self.data.len() as u16)?;
        write.write_all(&self.data)?;

        Ok(())
    }
}

impl MOHeader {
    fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.len() != 28 {
            return Err(Error::FormatError("Invalid header length".to_string()));
        }

        Ok(Self {
            cdr_reference: u32::from_be_bytes(TryFrom::try_from(&data[0..4]).unwrap()),
            imei: String::from_utf8_lossy(&data[4..19]).to_string(),
            session_status: SessionStatus::from_u8(data[19])?,
            momsn: u16::from_be_bytes(TryFrom::try_from(&data[20..22]).unwrap()),
            mtmsn: u16::from_be_bytes(TryFrom::try_from(&data[22..24]).unwrap()),
            time_of_session: Utc.timestamp_opt(
                u32::from_be_bytes(TryFrom::try_from(&data[24..28]).unwrap()) as i64, 0
            ).single().ok_or_else(|| {
                Error::FormatError("Invalid timestamp".to_string())
            })?
        })
    }
}

impl SessionStatus {
    fn from_u8(val: u8) -> Result<Self, Error> {
        match val {
            0 => Ok(Self::Successful),
            1 => Ok(Self::SuccessfulTooLarge),
            2 => Ok(Self::SuccessfulUnacceptableLocation),
            10 => Ok(Self::Timeout),
            11 => Ok(Self::TooLarge),
            13 => Ok(Self::RFLinkLost),
            14 => Ok(Self::ProtocolAnomaly),
            15 => Ok(Self::IMEIBlocked),
            _ => Err(Error::FormatError("Invalid status code".to_string()))
        }
    }
}

impl LocationInformation {
    fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.len() != 11 {
            return Err(Error::FormatError("Invalid location information length".to_string()));
        }

        let format_byte = data[0];
        let format_code = (format_byte & 0b00001100) >> 2;

        if format_code != 0 {
            return Err(Error::FormatError("Unsupported location format code".to_string()));
        }

        let north_south_indicator = ((format_byte & 0b00000010) >> 1) != 0;
        let east_west_indicator = (format_byte & 0b00000011) != 0;

        let mut latitude = data[1] as f32;
        latitude += u16::from_be_bytes(TryFrom::try_from(&data[2..4]).unwrap()) as f32 / 60000f32;
        let mut longitude = data[4] as f32;
        longitude += u16::from_be_bytes(TryFrom::try_from(&data[5..7]).unwrap()) as f32 / 60000f32;

        if north_south_indicator {
            latitude *= -1f32;
        }
        if east_west_indicator {
            longitude *= -1f32;
        }

        Ok(Self {
            latitude,
            longitude,
            cep_radius: u32::from_be_bytes(TryFrom::try_from(&data[7..11]).unwrap())
        })
    }
}

impl MOConfirmation {
    pub fn to_element(&self) -> Element {
        Element {
            id: 0x05,
            data: vec![if self.status { 0x01 } else { 0x00 }]
        }
    }
}

impl MTHeader {
    pub fn to_element(&self) -> Element {
        use byteorder::{WriteBytesExt, BigEndian};
        use std::io::Write;

        let mut data = std::io::Cursor::new(vec![]);

        let mut flags = 0;

        if self.flush_mt_queue {
            flags |= 1;
        }
        if self.send_ring_alert {
            flags |= 2;
        }
        if self.update_ssd_location {
            flags |= 8;
        }
        if self.high_priority_message {
            flags |= 16;
        }
        if self.assign_mtmsn {
            flags |= 32;
        }

        data.write_u32::<BigEndian>(self.client_message_id).unwrap();
        data.write_all(&self.imei.as_bytes()[0..15]).unwrap();
        data.write_u16::<BigEndian>(flags).unwrap();

        Element {
            id: 0x41,
            data: data.into_inner()
        }
    }
}

impl MTPayload {
    pub fn to_element(self) -> Element {
        Element {
            id: 0x42,
            data: self.data.clone()
        }
    }
}

impl MTPriority {
    pub fn to_element(&self) -> Element {
        Element {
            id: 0x46,
            data: self.level.to_be_bytes().to_vec()
        }
    }
}

impl MessageStatus {
    fn from_i8(val: i8) -> Result<Self, Error> {
        match val {
            0 => Ok(Self::SuccessfulNoPayload),
            x if 1 <= x && x <= 50 => Ok(Self::Successful(x as u8)),
            -1 => Ok(Self::InvalidIMEI),
            -2 => Ok(Self::UnknownIMEI),
            -3 => Ok(Self::TooLarge),
            -4 => Ok(Self::PayloadExpected),
            -5 => Ok(Self::QueueFull),
            -6 => Ok(Self::ResourcesUnavailable),
            -7 => Ok(Self::ProtocolViolation),
            -8 => Ok(Self::RingAlertsDisabled),
            -9 => Ok(Self::UnattachedIMEI),
            -10 => Ok(Self::IPBlocked),
            -11 => Ok(Self::MTMSNOutOfRange),
            _ => Err(Error::FormatError("Invalid message status".to_string()))
        }
    }
}

impl MTConfirmation {
    fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.len() != 25 {
            return Err(Error::FormatError("Invalid confirmation length".to_string()));
        }

        Ok(Self {
            client_message_id: u32::from_be_bytes(TryFrom::try_from(&data[0..4]).unwrap()),
            imei: String::from_utf8_lossy(&data[4..19]).to_string(),
            auto_id_reference: u32::from_be_bytes(TryFrom::try_from(&data[19..23]).unwrap()),
            message_status: MessageStatus::from_i8(data[24] as i8)?
        })
    }
}