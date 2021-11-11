use bitfield::bitfield;
use byteorder::{LittleEndian, ReadBytesExt};
use std::{convert::From, fmt, io, result};

pub mod error;
pub use error::LoraWanError;

#[derive(Debug)]
pub enum Direction {
    Uplink,
    Downlink,
}

#[derive(Debug)]
pub enum MType {
    JoinRequest,
    JoinAccept,
    UnconfirmedUp,
    UnconfirmedDown,
    ConfirmedUp,
    ConfirmedDown,
    Invalid(u8),
}

impl From<u8> for MType {
    fn from(v: u8) -> Self {
        match v {
            0b000 => MType::JoinRequest,
            0b001 => MType::JoinAccept,
            0b010 => MType::UnconfirmedUp,
            0b011 => MType::UnconfirmedDown,
            0b100 => MType::ConfirmedUp,
            0b101 => MType::ConfirmedDown,
            _ => MType::Invalid(v),
        }
    }
}

bitfield! {
    pub struct MHDR(u8);
    impl Debug;
    pub into MType, mtype, set_mtype: 7, 5;
    rfu, _: 4, 2;
    pub major, set_major: 1, 0;
}

impl MHDR {
    pub fn read(reader: &mut dyn io::Read) -> Result<Self, LoraWanError> {
        Ok(Self(reader.read_u8()?))
    }
}

#[derive(Debug)]
pub struct PHYPayload {
    pub mhdr: MHDR,
    pub payload: PHYPayloadFrame,
    pub mic: [u8; 4],
}

const JOIN_REQUEST_LEN: usize = 23;
const JOIN_ACCEPT_LEN: usize = 17;
const JOIN_ACCEPT_WITH_CFLIST_LEN: usize = 33;
const DATA_MIN_LEN: usize = 12;

impl PHYPayload {
    pub fn read(direction: Direction, reader: &mut dyn io::Read) -> Result<Self, LoraWanError> {
        let mhdr = MHDR::read(reader)?;
        let packet_type = mhdr.mtype();
        let mut data = vec![];
        reader.read_to_end(&mut data)?;

        let phy_len = data.len() + 1;
        let invalid = match packet_type {
            MType::JoinRequest => phy_len != JOIN_REQUEST_LEN,
            MType::JoinAccept => {
                phy_len != JOIN_ACCEPT_LEN && phy_len != JOIN_ACCEPT_WITH_CFLIST_LEN
            }
            MType::UnconfirmedUp
            | MType::UnconfirmedDown
            | MType::ConfirmedUp
            | MType::ConfirmedDown => phy_len < DATA_MIN_LEN,
            MType::Invalid(_) => false,
        };
        if invalid {
            return Err(LoraWanError::InvalidPacketSize(packet_type, phy_len));
        } else if let MType::Invalid(s) = packet_type {
            return Err(LoraWanError::InvalidPacketType(s));
        }

        // indexing with subtraction won't fail because of length checks above
        let mic = data.split_off(data.len() - 4);
        let mut payload = &data[..];
        let mut res = Self {
            mhdr,
            payload: PHYPayloadFrame::read(direction, packet_type, &mut payload)?,
            mic: [0; 4],
        };
        res.mic.copy_from_slice(&mic);
        Ok(res)
    }

    pub fn mtype(&self) -> MType {
        self.mhdr.mtype()
    }
}

#[derive(Debug)]
pub enum PHYPayloadFrame {
    MACPayload(MACPayload),
    JoinRequest(JoinRequest),
    JoinAccept(JoinAccept),
}

impl PHYPayloadFrame {
    pub fn read(
        direction: Direction,
        packet_type: MType,
        reader: &mut dyn io::Read,
    ) -> Result<Self, LoraWanError> {
        let res = match packet_type {
            MType::JoinRequest => Self::JoinRequest(JoinRequest::read(reader)?),
            MType::JoinAccept => Self::JoinAccept(JoinAccept::read(reader)?),
            _ => Self::MACPayload(MACPayload::read(packet_type, direction, reader)?),
        };
        Ok(res)
    }

    pub fn fcnt(&self) -> Option<u16> {
        match self {
            Self::MACPayload(payload) => Some(payload.fhdr.fcnt),
            _ => None,
        }
    }
}

pub struct Fhdr {
    pub dev_addr: u32,
    pub fctrl: FCtrl,
    pub fcnt: u16,
    pub fopts: Vec<u8>,
}

impl fmt::Debug for Fhdr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> result::Result<(), fmt::Error> {
        f.debug_struct("Fhdr")
            .field("dev_addr", &format_args!("{:#04x}", self.dev_addr))
            .field("fctrl", &self.fctrl)
            .field("fcnt", &self.fcnt)
            .field("fopts", &self.fopts)
            .finish()
    }
}

impl Fhdr {
    pub fn read(direction: Direction, reader: &mut dyn io::Read) -> Result<Self, LoraWanError> {
        let dev_addr = reader.read_u32::<LittleEndian>()?;
        let fctrl = FCtrl::read(direction, reader)?;
        let fcnt = reader.read_u16::<LittleEndian>()?;
        let mut fopts = Vec::with_capacity(fctrl.fopts_len().into());
        reader.read_exact(&mut fopts)?;
        let res = Self {
            dev_addr,
            fctrl,
            fcnt,
            fopts,
        };
        Ok(res)
    }
}

bitfield! {
    pub struct FCtrlUplink(u8);
    impl Debug;
    pub adr, set_adr: 7;
    pub adr_ack_req, set_addr_ack_req: 6;
    pub ack, set_ack: 5;
    pub fpending, set_fpending: 4;
    pub fopts_len, set_fopts_len:3, 0;
}

impl FCtrlUplink {
    pub fn read(reader: &mut dyn io::Read) -> Result<Self, LoraWanError> {
        Ok(Self(reader.read_u8()?))
    }
}

bitfield! {
    pub struct FCtrlDownlink(u8);
    impl Debug;
    pub adr, set_adr: 7;
    rfu, _: 6;
    pub ack, set_ack: 5;
    pub class_b, set_class_b: 4;
    pub fopts_len, set_fopts_len:3, 0;
}

impl FCtrlDownlink {
    pub fn read(reader: &mut dyn io::Read) -> Result<Self, LoraWanError> {
        Ok(Self(reader.read_u8()?))
    }
}

#[derive(Debug)]
pub enum FCtrl {
    Uplink(FCtrlUplink),
    Downlink(FCtrlDownlink),
}

impl FCtrl {
    pub fn fopts_len(&self) -> u8 {
        match self {
            FCtrl::Uplink(fctrl) => fctrl.fopts_len(),
            FCtrl::Downlink(fctrl) => fctrl.fopts_len(),
        }
    }

    pub fn read(direction: Direction, reader: &mut dyn io::Read) -> Result<Self, LoraWanError> {
        let res = match direction {
            Direction::Uplink => Self::Uplink(FCtrlUplink::read(reader)?),
            Direction::Downlink => Self::Downlink(FCtrlDownlink::read(reader)?),
        };
        Ok(res)
    }
}

#[derive(Debug)]
pub struct MACPayload {
    pub fhdr: Fhdr,
    pub fport: Option<u8>,
    pub payload: Option<FRMPayload>,
}

impl MACPayload {
    pub fn read(
        payload_type: MType,
        direction: Direction,
        reader: &mut dyn io::Read,
    ) -> Result<Self, LoraWanError> {
        let fhdr = Fhdr::read(direction, reader)?;
        let mut data = vec![];
        reader.read_to_end(&mut data)?;
        let (fport, payload) = match data.split_first() {
            Some((port, mut payload)) => (
                Some(*port),
                Some(FRMPayload::read(payload_type, &mut payload)?),
            ),
            _ => (None, None),
        };
        if fport == Some(0) && fhdr.fctrl.fopts_len() > 0 {
            return Err(LoraWanError::InvalidFPortForFopts);
        }
        let res = Self {
            fhdr,
            fport,
            payload,
        };
        Ok(res)
    }

    pub fn dev_addr(&self) -> u32 {
        self.fhdr.dev_addr
    }
}

#[derive(Debug)]
pub enum FRMPayload {
    UnconfirmedUp(Payload),
    UnconfirmedDown(Payload),
    ConfirmedUp(Payload),
    ConfirmedDown(Payload),
}

impl FRMPayload {
    pub fn read(payload_type: MType, reader: &mut dyn io::Read) -> Result<Self, LoraWanError> {
        let res = match payload_type {
            MType::UnconfirmedUp => Self::UnconfirmedUp(Payload::read(reader)?),
            MType::UnconfirmedDown => Self::UnconfirmedDown(Payload::read(reader)?),
            MType::ConfirmedUp => Self::ConfirmedUp(Payload::read(reader)?),
            MType::ConfirmedDown => Self::ConfirmedDown(Payload::read(reader)?),
            MType::Invalid(v) => return Err(LoraWanError::InvalidPacketType(v)),
            _ => unreachable!(),
        };
        Ok(res)
    }
}

#[derive(Debug)]
pub struct Payload(Vec<u8>);

impl Payload {
    pub fn read(reader: &mut dyn io::Read) -> Result<Self, LoraWanError> {
        let mut data = vec![];
        reader.read_to_end(&mut data)?;
        let res = Self(data);
        Ok(res)
    }
}

pub struct JoinRequest {
    pub app_eui: u64,
    pub dev_eui: u64,
    pub dev_nonce: [u8; 2],
}

impl fmt::Debug for JoinRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> result::Result<(), fmt::Error> {
        f.debug_struct("JoinRequest")
            .field("app_eui", &format_args!("{:#08x}", self.app_eui))
            .field("dev_eui", &format_args!("{:#08x}", self.dev_eui))
            .field("dev_nonce", &self.dev_nonce)
            .finish()
    }
}

impl JoinRequest {
    pub fn read(reader: &mut dyn io::Read) -> Result<Self, LoraWanError> {
        let mut res = Self {
            app_eui: reader.read_u64::<LittleEndian>()?,
            dev_eui: reader.read_u64::<LittleEndian>()?,
            dev_nonce: [0; 2],
        };
        reader.read_exact(&mut res.dev_nonce)?;
        Ok(res)
    }
}

#[derive(Debug)]
pub struct JoinAccept {
    pub app_nonce: [u8; 3],
    pub net_id: [u8; 3],
    pub dev_addr: u32,
    pub dl_settings: u8,
    pub rx_delay: u8,
    // cf_list: Option<CFList>,
}

impl JoinAccept {
    pub fn read(reader: &mut dyn io::Read) -> Result<Self, LoraWanError> {
        let mut app_nonce = [0u8; 3];
        let mut net_id = [0u8; 3];
        reader.read_exact(&mut app_nonce)?;
        reader.read_exact(&mut net_id)?;
        let res = Self {
            app_nonce,
            net_id,
            dev_addr: reader.read_u32::<LittleEndian>()?,
            dl_settings: reader.read_u8()?,
            rx_delay: reader.read_u8()?,
        };
        Ok(res)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use base64;

    #[test]
    fn test_read() {
        let mut data = &base64::decode("IL1ciMu7b3ZOP5Q1cBA7isI=").unwrap()[..];
        let payload = PHYPayload::read(Direction::Uplink, &mut data).unwrap();
        eprintln!("PAYLOAD {:?}", payload);
    }
}
