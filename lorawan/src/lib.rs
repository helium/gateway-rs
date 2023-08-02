use bitfield::bitfield;
use bytes::{Buf, BufMut, Bytes};
use std::{convert::From, fmt, mem::size_of, result};

pub mod error;
pub use error::LoraWanError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Uplink,
    Downlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MType {
    JoinRequest,
    JoinAccept,
    UnconfirmedUp,
    UnconfirmedDown,
    ConfirmedUp,
    ConfirmedDown,
    Proprietary,
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
            0b111 => MType::Proprietary,
            _ => MType::Invalid(v),
        }
    }
}

impl From<MType> for u8 {
    fn from(m: MType) -> Self {
        match m {
            MType::JoinRequest => 0b000,
            MType::JoinAccept => 0b001,
            MType::UnconfirmedUp => 0b010,
            MType::UnconfirmedDown => 0b011,
            MType::ConfirmedUp => 0b100,
            MType::ConfirmedDown => 0b101,
            MType::Proprietary => 0b111,
            MType::Invalid(v) => v,
        }
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct MHDR(u8);
    impl Debug;
    pub from into MType, mtype, set_mtype: 7, 5;
    rfu, _: 4, 2;
    pub major, set_major: 1, 0;
}

impl MHDR {
    pub fn read(reader: &mut dyn Buf) -> Result<Self, LoraWanError> {
        Ok(Self(reader.get_u8()))
    }

    pub fn write(self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        output.put_u8(self.0);
        Ok(1)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PHYPayload {
    pub mhdr: MHDR,
    pub payload: PHYPayloadFrame,
    pub mic: Option<[u8; 4]>,
}

const JOIN_REQUEST_LEN: usize = 23;
const JOIN_ACCEPT_LEN: usize = 17;
const JOIN_ACCEPT_WITH_CFLIST_LEN: usize = 33;
const DATA_MIN_LEN: usize = 12;

impl PHYPayload {
    pub fn proprietary(payload: &[u8]) -> Self {
        PHYPayload {
            mhdr: {
                let mut mhdr = MHDR(0);
                mhdr.set_mtype(MType::Proprietary);
                mhdr
            },
            payload: PHYPayloadFrame::Proprietary(Bytes::copy_from_slice(payload)),
            mic: None,
        }
    }

    pub fn read(direction: Direction, reader: &mut dyn Buf) -> Result<Self, LoraWanError> {
        let mhdr = MHDR::read(reader)?;
        let version = mhdr.major();
        if version != 0 {
            return Err(LoraWanError::InvalidPacketVersion(version));
        }
        let packet_type = mhdr.mtype();
        let mut data = reader.copy_to_bytes(reader.remaining());

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
            // proprietary frames have unknown minimum length
            MType::Proprietary => false,
            // all invalid MType are invalid
            MType::Invalid(_) => true,
        };
        if invalid {
            return Err(LoraWanError::InvalidPacketSize(packet_type, phy_len));
        } else if let MType::Invalid(s) = packet_type {
            return Err(LoraWanError::InvalidPacketType(s));
        }

        // indexing with subtraction won't fail because of length checks above
        // Proprietary frames are assumed to take over the mic bytes
        let mic = if packet_type != MType::Proprietary {
            let mut mic_bytes = [0u8; 4];
            mic_bytes.copy_from_slice(&data.split_off(data.len() - 4));
            Some(mic_bytes)
        } else {
            None
        };

        let res = Self {
            mhdr,
            payload: PHYPayloadFrame::read(direction, packet_type, &mut data)?,
            mic,
        };
        Ok(res)
    }

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        let mut written = 0_usize;
        written += self.mhdr.write(output)?;
        written += self.payload.write(output)?;
        if let Some(mic) = self.mic {
            output.put_slice(&mic);
            written += mic.len();
        }
        Ok(written)
    }

    pub fn mtype(&self) -> MType {
        self.mhdr.mtype()
    }
}

impl TryFrom<PHYPayload> for Vec<u8> {
    type Error = LoraWanError;
    fn try_from(value: PHYPayload) -> Result<Self, Self::Error> {
        let mut data = vec![];
        value.write(&mut data)?;
        Ok(data)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PHYPayloadFrame {
    MACPayload(MACPayload),
    JoinRequest(JoinRequest),
    JoinAccept(JoinAccept),
    Proprietary(Bytes),
}

impl PHYPayloadFrame {
    pub fn read(
        direction: Direction,
        packet_type: MType,
        reader: &mut dyn Buf,
    ) -> Result<Self, LoraWanError> {
        let res = match packet_type {
            MType::JoinRequest => Self::JoinRequest(JoinRequest::read(reader)?),
            MType::JoinAccept => Self::JoinAccept(JoinAccept::read(reader)?),
            MType::Proprietary => {
                let proprietary_payload = reader.copy_to_bytes(reader.remaining());
                Self::Proprietary(proprietary_payload)
            }
            _ => Self::MACPayload(MACPayload::read(packet_type, direction, reader)?),
        };
        Ok(res)
    }

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        match self {
            Self::MACPayload(mp) => mp.write(output),
            Self::JoinRequest(jr) => jr.write(output),
            Self::JoinAccept(ja) => ja.write(output),
            Self::Proprietary(v) => {
                output.put_slice(v);
                Ok(v.len())
            }
        }
    }

    pub fn fcnt(&self) -> Option<u16> {
        match self {
            Self::MACPayload(payload) => Some(payload.fhdr.fcnt),
            _ => None,
        }
    }
}

#[derive(PartialEq, Eq, Clone)]
pub struct Fhdr {
    pub dev_addr: u32,
    pub fctrl: FCtrl,
    pub fcnt: u16,
    pub fopts: Bytes,
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
    pub fn read(direction: Direction, reader: &mut dyn Buf) -> Result<Self, LoraWanError> {
        let dev_addr = reader.get_u32_le();
        let fctrl = FCtrl::read(direction, reader)?;
        let fcnt = reader.get_u16_le();
        let fopts = reader.copy_to_bytes(fctrl.fopts_len());
        let res = Self {
            dev_addr,
            fctrl,
            fcnt,
            fopts,
        };
        Ok(res)
    }

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        let mut written = 0;
        output.put_u32_le(self.dev_addr);
        written += size_of::<u32>();
        output.put_u32_le(self.dev_addr);
        written += self.fctrl.write(output)?;
        output.put_u16_le(self.fcnt);
        written += size_of::<u16>();
        output.put_slice(&self.fopts);
        written += self.fopts.len();
        Ok(written)
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct FCtrlUplink(u8);
    impl Debug;
    pub adr, set_adr: 7;
    pub adr_ack_req, set_addr_ack_req: 6;
    pub ack, set_ack: 5;
    pub fpending, set_fpending: 4;
    pub fopts_len, set_fopts_len:3, 0;
}

impl FCtrlUplink {
    pub fn read(reader: &mut dyn Buf) -> Result<Self, LoraWanError> {
        Ok(Self(reader.get_u8()))
    }

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        output.put_u8(self.0);
        Ok(size_of::<Self>())
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct FCtrlDownlink(u8);
    impl Debug;
    pub adr, set_adr: 7;
    rfu, _: 6;
    pub ack, set_ack: 5;
    pub class_b, set_class_b: 4;
    pub fopts_len, set_fopts_len:3, 0;
}

impl FCtrlDownlink {
    pub fn read(reader: &mut dyn Buf) -> Result<Self, LoraWanError> {
        Ok(Self(reader.get_u8()))
    }

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        output.put_u8(self.0);
        Ok(size_of::<Self>())
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum FCtrl {
    Uplink(FCtrlUplink),
    Downlink(FCtrlDownlink),
}

impl FCtrl {
    pub fn fopts_len(&self) -> usize {
        match self {
            FCtrl::Uplink(fctrl) => fctrl.fopts_len().into(),
            FCtrl::Downlink(fctrl) => fctrl.fopts_len().into(),
        }
    }

    pub fn read(direction: Direction, reader: &mut dyn Buf) -> Result<Self, LoraWanError> {
        let res = match direction {
            Direction::Uplink => Self::Uplink(FCtrlUplink::read(reader)?),
            Direction::Downlink => Self::Downlink(FCtrlDownlink::read(reader)?),
        };
        Ok(res)
    }

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        match self {
            Self::Uplink(ul) => ul.write(output),
            Self::Downlink(dl) => dl.write(output),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MACPayload {
    pub fhdr: Fhdr,
    pub fport: Option<u8>,
    pub payload: Option<FRMPayload>,
}

impl MACPayload {
    pub fn read(
        payload_type: MType,
        direction: Direction,
        reader: &mut dyn Buf,
    ) -> Result<Self, LoraWanError> {
        let fhdr = Fhdr::read(direction, reader)?;
        let data = reader.copy_to_bytes(reader.remaining());
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

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        let mut written = 0;
        written += self.fhdr.write(output)?;
        written += match self.fport {
            Some(fp) => {
                output.put_u8(fp);
                size_of::<u8>()
            }
            None => 0,
        };
        written += match &self.payload {
            Some(p) => p.write(output)?,
            None => 0,
        };
        Ok(written)
    }

    pub fn dev_addr(&self) -> u32 {
        self.fhdr.dev_addr
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum FRMPayload {
    UnconfirmedUp(Payload),
    UnconfirmedDown(Payload),
    ConfirmedUp(Payload),
    ConfirmedDown(Payload),
}

impl FRMPayload {
    pub fn read(payload_type: MType, reader: &mut dyn Buf) -> Result<Self, LoraWanError> {
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

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        match self {
            Self::UnconfirmedUp(p) => p.write(output),
            Self::UnconfirmedDown(p) => p.write(output),
            Self::ConfirmedUp(p) => p.write(output),
            Self::ConfirmedDown(p) => p.write(output),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Payload(Bytes);

impl Payload {
    pub fn read(reader: &mut dyn Buf) -> Result<Self, LoraWanError> {
        let data = reader.copy_to_bytes(reader.remaining());
        Ok(Self(data))
    }

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        output.put_slice(&self.0);
        Ok(self.0.len())
    }
}

#[derive(PartialEq, Eq, Clone)]
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
    pub fn read(reader: &mut dyn Buf) -> Result<Self, LoraWanError> {
        // TODO: Reader length check
        let mut res = Self {
            app_eui: reader.get_u64_le(),
            dev_eui: reader.get_u64_le(),
            dev_nonce: [0; 2],
        };
        reader.copy_to_slice(&mut res.dev_nonce);
        Ok(res)
    }

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        output.put_u64_le(self.app_eui);
        output.put_u64_le(self.dev_eui);
        output.put_slice(&self.dev_nonce);
        Ok(size_of::<Self>())
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct JoinAccept {
    pub app_nonce: [u8; 3],
    pub net_id: [u8; 3],
    pub dev_addr: u32,
    pub dl_settings: u8,
    pub rx_delay: u8,
    // cf_list: Option<CFList>,
}

impl JoinAccept {
    pub fn read(reader: &mut dyn Buf) -> Result<Self, LoraWanError> {
        // TODO: Reader length check
        let mut app_nonce = [0u8; 3];
        let mut net_id = [0u8; 3];
        reader.copy_to_slice(&mut app_nonce);
        reader.copy_to_slice(&mut net_id);
        let res = Self {
            app_nonce,
            net_id,
            dev_addr: reader.get_u32_le(),
            dl_settings: reader.get_u8(),
            rx_delay: reader.get_u8(),
        };
        Ok(res)
    }

    pub fn write(&self, output: &mut dyn BufMut) -> Result<usize, LoraWanError> {
        output.put_slice(&self.app_nonce);
        output.put_slice(&self.net_id);
        output.put_u32_le(self.dev_addr);
        output.put_u8(self.dl_settings);
        output.put_u8(self.rx_delay);
        Ok(size_of::<Self>())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use base64;

    #[test]
    fn test_read_write_roundtrip() {
        use base64::Engine;
        let data_a = base64::engine::general_purpose::STANDARD
            .decode("IL1ciMu7b3ZOP5Q1cBA7isI=")
            .unwrap();
        let payload_a = PHYPayload::read(Direction::Uplink, &mut &data_a[..]).unwrap();
        let mut data_b = Vec::with_capacity(data_a.len());
        payload_a.write(&mut data_b).unwrap();
        assert_eq!(data_a, data_b);
    }

    #[test]
    fn test_packets() {
        for (routing, data) in mk_test_packets() {
            let expected_routing = Routing::try_from(data).expect("routing");
            assert_eq!(routing, expected_routing);
        }
    }

    impl TryFrom<&[u8]> for Routing {
        type Error = LoraWanError;
        fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
            match PHYPayload::read(Direction::Uplink, &mut &value[..]) {
                Err(_err) => Ok(Self::Invalid),
                Ok(payload) => payload.try_into(),
            }
        }
    }

    impl TryFrom<PHYPayload> for Routing {
        type Error = LoraWanError;
        fn try_from(value: PHYPayload) -> Result<Self, Self::Error> {
            fn get_dev_addr(mtype: MType, payload: &PHYPayload) -> Result<u32, LoraWanError> {
                match payload.payload {
                    PHYPayloadFrame::MACPayload(MACPayload {
                        fhdr: Fhdr { dev_addr, .. },
                        ..
                    }) => Ok(dev_addr),
                    _ => return Err(LoraWanError::InvalidPacketType(mtype.into())),
                }
            }

            match value.mtype() {
                MType::UnconfirmedUp => {
                    let dev_addr = get_dev_addr(value.mtype(), &value)?;
                    Ok(Self::Unconfirmed {
                        devaddr: format!("{:06X}", dev_addr),
                    })
                }
                MType::ConfirmedUp => {
                    let dev_addr = get_dev_addr(value.mtype(), &value)?;
                    Ok(Self::Confirmed {
                        devaddr: format!("{:06X}", dev_addr),
                    })
                }
                MType::JoinRequest => {
                    let (dev_eui, app_eui) = match value.payload {
                        PHYPayloadFrame::JoinRequest(JoinRequest {
                            dev_eui, app_eui, ..
                        }) => (dev_eui, app_eui),
                        _ => return Err(LoraWanError::InvalidPacketType(value.mtype().into())),
                    };
                    Ok(Self::Join {
                        eui: format!("{:06X}", app_eui),
                        dev: format!("{:06X}", dev_eui),
                    })
                }
                _ => Ok(Self::Invalid),
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Routing {
        Unconfirmed { devaddr: String },
        Confirmed { devaddr: String },
        Join { eui: String, dev: String },
        Invalid,
    }

    fn mk_test_packets() -> Vec<(Routing, &'static [u8])> {
        vec![
            (
                Routing::Unconfirmed {
                    devaddr: "65A547".to_string(),
                },
                &[
                    64, 71, 165, 101, 0, 128, 130, 41, 2, 214, 3, 27, 61, 140, 165, 211, 143, 196,
                    1, 134, 56, 31, 122, 222,
                ],
            ),
            (Routing::Invalid, &[4, 217, 181, 6, 11, 130, 2, 254, 1]),
            (
                Routing::Join {
                    eui: "70B3D5B02000088D".to_string(),
                    dev: "70B3D5B020038C7F".to_string(),
                },
                &[
                    0, 141, 8, 0, 32, 176, 213, 179, 112, 127, 140, 3, 32, 176, 213, 179, 112, 135,
                    15, 125, 90, 77, 199,
                ],
            ),
            (
                Routing::Confirmed {
                    devaddr: "127B3F4".to_string(),
                },
                &[
                    128, 244, 179, 39, 1, 128, 27, 0, 61, 112, 100, 42, 151, 154, 203, 136, 193,
                    200, 210, 165,
                ],
            ),
            (
                Routing::Join {
                    eui: "1122334455667799".to_string(),
                    dev: "956906000056AD".to_string(),
                },
                &[
                    0, 153, 119, 102, 85, 68, 51, 34, 17, 173, 86, 0, 0, 6, 105, 149, 0, 151, 197,
                    232, 148, 220, 26,
                ],
            ),
            (Routing::Invalid, &[4, 73, 201, 4, 152, 149, 2, 254, 1]),
            (Routing::Invalid, &[4, 214, 159, 4, 116, 253, 2, 254, 1]),
            (
                Routing::Unconfirmed {
                    devaddr: "90CC175".to_string(),
                },
                &[
                    64, 117, 193, 12, 9, 128, 255, 254, 5, 4, 240, 62, 237, 70, 223, 10, 6, 103,
                    172, 117, 113, 137, 253, 157, 62, 152, 146, 62,
                ],
            ),
            (
                Routing::Unconfirmed {
                    devaddr: "203D5C8".to_string(),
                },
                &[
                    64, 200, 213, 3, 2, 128, 11, 124, 1, 38, 51, 2, 5, 95, 101, 161, 40, 44, 86,
                    116, 134, 134, 205, 127, 80, 215, 216, 107, 195, 105, 179, 202, 251, 251, 103,
                    113, 108, 15, 139, 26, 35, 190, 230, 163, 135, 83, 179,
                ],
            ),
        ]
    }
}
