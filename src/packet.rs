use crate::{DecodeError, Error, PublicKey, Region, Result};
use helium_proto::services::{
    poc_lora,
    router::{PacketRouterPacketDownV1, PacketRouterPacketUpV1},
};
use lorawan::{Direction, PHYPayloadFrame, MHDR};
use semtech_udp::{
    pull_resp::{self, PhyData, Time},
    push_data::{self, CRC},
    CodingRate, DataRate, Modulation,
};
use sha2::{Digest, Sha256};
use std::{
    convert::TryFrom,
    fmt,
    ops::Deref,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq)]
pub struct PacketUp(PacketRouterPacketUpV1);

#[derive(Debug, Clone)]
pub struct PacketDown(PacketRouterPacketDownV1);

impl Deref for PacketUp {
    type Target = PacketRouterPacketUpV1;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<PacketUp> for PacketRouterPacketUpV1 {
    fn from(value: PacketUp) -> Self {
        value.0
    }
}
impl From<&PacketUp> for PacketRouterPacketUpV1 {
    fn from(value: &PacketUp) -> Self {
        value.0.clone()
    }
}

impl From<PacketRouterPacketDownV1> for PacketDown {
    fn from(value: PacketRouterPacketDownV1) -> Self {
        Self(value)
    }
}

impl fmt::Display for PacketUp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "@{} us, {:.2} MHz, {:?}, snr: {}, rssi: {}, len: {}",
            self.0.timestamp,
            self.0.frequency,
            self.0.datarate(),
            self.0.snr,
            self.0.rssi,
            self.0.payload.len()
        ))
    }
}

impl TryFrom<PacketUp> for poc_lora::LoraWitnessReportReqV1 {
    type Error = Error;
    fn try_from(value: PacketUp) -> Result<Self> {
        let report = poc_lora::LoraWitnessReportReqV1 {
            data: vec![],
            tmst: value.0.timestamp as u32,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(Error::from)?
                .as_nanos() as u64,
            signal: value.0.rssi * 10,
            snr: (value.0.snr * 10.0) as i32,
            frequency: value.0.frequency as u64,
            datarate: value.0.datarate,
            pub_key: vec![],
            signature: vec![],
        };
        Ok(report)
    }
}

impl PacketUp {
    pub fn from_rxpk(rxpk: push_data::RxPk, gateway: &PublicKey, region: Region) -> Result<Self> {
        match rxpk.get_crc_status() {
            CRC::OK => (),
            CRC::Disabled => return Err(DecodeError::crc_disabled()),
            CRC::Fail => return Err(DecodeError::crc_invalid()),
        }

        let rssi = rxpk
            .get_signal_rssi()
            .unwrap_or_else(|| rxpk.get_channel_rssi());

        let packet = PacketRouterPacketUpV1 {
            rssi,
            timestamp: *rxpk.get_timestamp() as u64,
            payload: rxpk.get_data().to_vec(),
            frequency: to_hz(*rxpk.get_frequency()) as u32,
            datarate: datarate::to_proto(rxpk.get_datarate())? as i32,
            snr: rxpk.get_snr(),
            region: region.into(),
            hold_time: 0,
            gateway: gateway.into(),
            signature: vec![],
        };
        Ok(Self(packet))
    }

    pub fn is_potential_beacon(&self) -> bool {
        Self::parse_header(self.payload())
            .map(|header| {
                header.mtype() == lorawan::MType::Proprietary
                    && self.payload().len() == beacon::BEACON_PAYLOAD_SIZE + Self::header_size()
            })
            .unwrap_or(false)
    }

    pub fn is_uplink(&self) -> bool {
        // An uplinkable packet is a parseable lorawan uplink frame which is not
        // a proprietary frame
        Self::parse_frame(Direction::Uplink, self.payload())
            .map(|frame| {
                !matches!(
                    frame,
                    PHYPayloadFrame::Proprietary(_) | PHYPayloadFrame::JoinAccept(_),
                )
            })
            .unwrap_or(false)
    }

    pub fn payload(&self) -> &[u8] {
        &self.0.payload
    }

    pub fn parse_header(payload: &[u8]) -> Result<MHDR> {
        use std::io::Cursor;
        lorawan::MHDR::read(&mut Cursor::new(payload)).map_err(Error::from)
    }

    pub fn header_size() -> usize {
        std::mem::size_of::<MHDR>()
    }

    pub fn parse_frame(direction: lorawan::Direction, payload: &[u8]) -> Result<PHYPayloadFrame> {
        use std::io::Cursor;
        lorawan::PHYPayload::read(direction, &mut Cursor::new(payload))
            .map(|p| p.payload)
            .map_err(Error::from)
    }

    pub fn hash(&self) -> Vec<u8> {
        Sha256::digest(&self.0.payload).to_vec()
    }
}

impl PacketDown {
    pub fn to_rx1_pull_resp(&self, tx_power: u32) -> Result<pull_resp::TxPk> {
        let rx1 = self.0.rx1.as_ref().ok_or_else(DecodeError::no_rx1_window)?;
        let time = if rx1.immediate {
            Time::immediate()
        } else {
            Time::by_tmst(rx1.timestamp as u32)
        };
        self.inner_to_pull_resp(
            time,
            rx1.frequency,
            datarate::from_proto(rx1.datarate())?,
            tx_power,
        )
    }

    pub fn to_rx2_pull_resp(&self, tx_power: u32) -> Result<Option<pull_resp::TxPk>> {
        let rx2 = match self.0.rx2.as_ref() {
            Some(window) => window,
            None => return Ok(None),
        };

        self.inner_to_pull_resp(
            Time::by_tmst(rx2.timestamp as u32),
            rx2.frequency,
            datarate::from_proto(rx2.datarate())?,
            tx_power,
        )
        .map(Some)
    }

    fn inner_to_pull_resp(
        &self,
        time: Time,
        frequency_hz: u32,
        datarate: DataRate,
        tx_power: u32,
    ) -> Result<pull_resp::TxPk> {
        Ok(pull_resp::TxPk {
            time,
            ipol: true,
            modu: Modulation::LORA,
            codr: CodingRate::_4_5,
            datr: datarate,
            // for normal lorawan packets we're not selecting different frequencies
            // like we are for PoC
            freq: to_mhz(frequency_hz),
            data: PhyData::new(self.0.payload.clone()),
            powe: tx_power as u64,
            rfch: 0,
            fdev: None,
            prea: None,
            ncrc: None,
        })
    }
}

pub(crate) fn to_hz<M: Into<f64>>(mhz: M) -> u64 {
    (mhz.into() * 1_000_000f64).trunc() as u64
}

pub(crate) fn to_mhz<H: Into<f64>>(hz: H) -> f64 {
    hz.into() / 1_000_000.0
}

pub(crate) mod datarate {
    use super::{DecodeError, Result};
    use helium_proto::DataRate as ProtoRate;
    use semtech_udp::{Bandwidth, DataRate, SpreadingFactor};

    pub fn from_proto(rate: ProtoRate) -> Result<DataRate> {
        let (spreading_factor, bandwidth) = match rate {
            ProtoRate::Sf12bw125 => (SpreadingFactor::SF12, Bandwidth::BW125),
            ProtoRate::Sf11bw125 => (SpreadingFactor::SF11, Bandwidth::BW125),
            ProtoRate::Sf10bw125 => (SpreadingFactor::SF10, Bandwidth::BW125),
            ProtoRate::Sf9bw125 => (SpreadingFactor::SF9, Bandwidth::BW125),
            ProtoRate::Sf8bw125 => (SpreadingFactor::SF8, Bandwidth::BW125),
            ProtoRate::Sf7bw125 => (SpreadingFactor::SF7, Bandwidth::BW125),

            ProtoRate::Sf12bw250 => (SpreadingFactor::SF12, Bandwidth::BW250),
            ProtoRate::Sf11bw250 => (SpreadingFactor::SF11, Bandwidth::BW250),
            ProtoRate::Sf10bw250 => (SpreadingFactor::SF10, Bandwidth::BW250),
            ProtoRate::Sf9bw250 => (SpreadingFactor::SF9, Bandwidth::BW250),
            ProtoRate::Sf8bw250 => (SpreadingFactor::SF8, Bandwidth::BW250),
            ProtoRate::Sf7bw250 => (SpreadingFactor::SF7, Bandwidth::BW250),

            ProtoRate::Sf12bw500 => (SpreadingFactor::SF12, Bandwidth::BW500),
            ProtoRate::Sf11bw500 => (SpreadingFactor::SF11, Bandwidth::BW500),
            ProtoRate::Sf10bw500 => (SpreadingFactor::SF10, Bandwidth::BW500),
            ProtoRate::Sf9bw500 => (SpreadingFactor::SF9, Bandwidth::BW500),
            ProtoRate::Sf8bw500 => (SpreadingFactor::SF8, Bandwidth::BW500),
            ProtoRate::Sf7bw500 => (SpreadingFactor::SF7, Bandwidth::BW500),

            ProtoRate::Lrfhss2bw137
            | ProtoRate::Lrfhss1bw336
            | ProtoRate::Lrfhss1bw137
            | ProtoRate::Lrfhss2bw336
            | ProtoRate::Lrfhss1bw1523
            | ProtoRate::Lrfhss2bw1523
            | ProtoRate::Fsk50 => {
                return Err(DecodeError::invalid_data_rate("unsupported".to_string()))
            }
        };
        Ok(DataRate::new(spreading_factor, bandwidth))
    }

    pub fn to_proto(rate: DataRate) -> Result<ProtoRate> {
        let rate = match (rate.spreading_factor(), rate.bandwidth()) {
            (SpreadingFactor::SF12, Bandwidth::BW125) => ProtoRate::Sf12bw125,
            (SpreadingFactor::SF11, Bandwidth::BW125) => ProtoRate::Sf11bw125,
            (SpreadingFactor::SF10, Bandwidth::BW125) => ProtoRate::Sf10bw125,
            (SpreadingFactor::SF9, Bandwidth::BW125) => ProtoRate::Sf9bw125,
            (SpreadingFactor::SF8, Bandwidth::BW125) => ProtoRate::Sf8bw125,
            (SpreadingFactor::SF7, Bandwidth::BW125) => ProtoRate::Sf7bw125,

            (SpreadingFactor::SF12, Bandwidth::BW250) => ProtoRate::Sf12bw250,
            (SpreadingFactor::SF11, Bandwidth::BW250) => ProtoRate::Sf11bw250,
            (SpreadingFactor::SF10, Bandwidth::BW250) => ProtoRate::Sf10bw250,
            (SpreadingFactor::SF9, Bandwidth::BW250) => ProtoRate::Sf9bw250,
            (SpreadingFactor::SF8, Bandwidth::BW250) => ProtoRate::Sf8bw250,
            (SpreadingFactor::SF7, Bandwidth::BW250) => ProtoRate::Sf7bw250,

            (SpreadingFactor::SF12, Bandwidth::BW500) => ProtoRate::Sf12bw500,
            (SpreadingFactor::SF11, Bandwidth::BW500) => ProtoRate::Sf11bw500,
            (SpreadingFactor::SF10, Bandwidth::BW500) => ProtoRate::Sf10bw500,
            (SpreadingFactor::SF9, Bandwidth::BW500) => ProtoRate::Sf9bw500,
            (SpreadingFactor::SF8, Bandwidth::BW500) => ProtoRate::Sf8bw500,
            (SpreadingFactor::SF7, Bandwidth::BW500) => ProtoRate::Sf7bw500,

            (SpreadingFactor::SF6, _) | (SpreadingFactor::SF5, _) => {
                return Err(DecodeError::invalid_data_rate(rate.to_string()))
            }
        };
        Ok(rate)
    }
}
