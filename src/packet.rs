use crate::{error::DecodeError, Error, Result};
use helium_proto::{
    packet::PacketType,
    routing_information::Data as RoutingData,
    services::{
        poc_lora,
        router::{PacketRouterPacketDownV1, PacketRouterPacketUpV1},
    },
    DataRate as ProtoDataRate, Eui, RoutingInformation,
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
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone)]
pub struct Packet(helium_proto::Packet);

impl Deref for Packet {
    type Target = helium_proto::Packet;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "@{} us, {:.2} MHz, {:?}, snr: {}, rssi: {}, len: {}",
            self.0.timestamp,
            self.0.frequency,
            DataRate::from_str(&self.0.datarate),
            self.0.snr,
            self.0.signal_strength,
            self.0.payload.len()
        ))
    }
}

impl TryFrom<push_data::RxPk> for Packet {
    type Error = Error;

    fn try_from(rxpk: push_data::RxPk) -> Result<Self> {
        if rxpk.get_crc_status() == &CRC::OK {
            let rssi = rxpk
                .get_signal_rssi()
                .unwrap_or_else(|| rxpk.get_channel_rssi());
            let packet = helium_proto::Packet {
                r#type: PacketType::Lorawan.into(),
                signal_strength: rssi as f32,
                snr: rxpk.get_snr(),
                frequency: *rxpk.get_frequency() as f32,
                // TODO: add `datetime` field here in the helium_proto::Packet definition
                // and set the value to *rxpk.get_time(), converted from Option<String> to u64
                timestamp: *rxpk.get_timestamp() as u64,
                datarate: rxpk.get_datarate().to_string(),
                routing: Self::routing_information(&Self::parse_frame(
                    lorawan::Direction::Uplink,
                    rxpk.get_data(),
                )?)?,
                payload: rxpk.get_data().to_vec(),
                rx2_window: None,
                oui: 0,
            };
            Ok(Self(packet))
        } else {
            Err(DecodeError::invalid_crc())
        }
    }
}

impl TryFrom<PacketRouterPacketDownV1> for Packet {
    type Error = Error;

    fn try_from(pr_down: PacketRouterPacketDownV1) -> Result<Self> {
        let window = pr_down.rx1.ok_or_else(DecodeError::no_rx1_window)?;
        let datarate = helium_proto::DataRate::from_i32(window.datarate)
            .ok_or_else(DecodeError::no_data_rate)?;
        let packet = helium_proto::Packet {
            oui: 0,
            r#type: PacketType::Lorawan.into(),
            payload: pr_down.payload,
            timestamp: window.timestamp,
            signal_strength: 0.0,
            frequency: window.frequency as f32 / 1_000_000.0,
            datarate: datarate.to_string(),
            snr: 0.0,
            routing: None,
            rx2_window: pr_down.rx2.map(|window| helium_proto::Window {
                timestamp: window.timestamp,
                frequency: window.frequency as f32 / 1_000_000.0,
                datarate: window.datarate.to_string(),
            }),
        };
        Ok(Self(packet))
    }
}

impl TryFrom<Packet> for PacketRouterPacketUpV1 {
    type Error = Error;
    fn try_from(value: Packet) -> Result<Self> {
        Ok(Self {
            payload: value.payload.clone(),
            timestamp: value.timestamp,
            rssi: value.signal_strength as i32,
            frequency: (value.frequency * 1_000_000.0) as u32,
            datarate: ProtoDataRate::from_str(&value.datarate)? as i32,
            snr: value.snr,
            region: 0,
            hold_time: 0,
            gateway: vec![],
            signature: vec![],
        })
    }
}

impl TryFrom<Packet> for poc_lora::LoraWitnessReportReqV1 {
    type Error = Error;
    fn try_from(value: Packet) -> Result<Self> {
        let payload = match Packet::parse_frame(Direction::Uplink, value.payload()) {
            Ok(PHYPayloadFrame::Proprietary(payload)) => payload,
            _ => return Err(DecodeError::not_beacon()),
        };
        let dr = ProtoDataRate::from_str(&value.datarate)
            .map_err(|_| DecodeError::invalid_beacon_data_rate(value.datarate.clone()))?;
        let report = poc_lora::LoraWitnessReportReqV1 {
            pub_key: vec![],
            data: payload,
            tmst: value.timestamp as u32,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(Error::from)?
                .as_nanos() as u64,
            signal: (value.signal_strength * 10.0) as i32,
            snr: (value.snr * 10.0) as i32,
            frequency: to_hz(value.frequency),
            datarate: dr as i32,
            signature: vec![],
        };
        Ok(report)
    }
}

impl From<helium_proto::Packet> for Packet {
    fn from(v: helium_proto::Packet) -> Self {
        Self(v)
    }
}

impl Packet {
    pub fn routing(&self) -> &Option<RoutingInformation> {
        &self.0.routing
    }

    pub fn to_packet(self) -> helium_proto::Packet {
        self.0
    }

    pub fn payload(&self) -> &[u8] {
        &self.0.payload
    }

    pub fn routing_information(frame: &PHYPayloadFrame) -> Result<Option<RoutingInformation>> {
        let routing_data = match frame {
            PHYPayloadFrame::JoinRequest(request) => Some(RoutingData::Eui(Eui {
                deveui: request.dev_eui,
                appeui: request.app_eui,
            })),
            PHYPayloadFrame::MACPayload(mac_payload) => {
                Some(RoutingData::Devaddr(mac_payload.dev_addr()))
            }
            _ => return Ok(None),
        };
        Ok(routing_data.map(|r| RoutingInformation { data: Some(r) }))
    }

    pub fn parse_frame(direction: lorawan::Direction, payload: &[u8]) -> Result<PHYPayloadFrame> {
        use std::io::Cursor;
        lorawan::PHYPayload::read(direction, &mut Cursor::new(payload))
            .map(|p| p.payload)
            .map_err(Error::from)
    }

    pub fn parse_header(payload: &[u8]) -> Result<MHDR> {
        use std::io::Cursor;
        lorawan::MHDR::read(&mut Cursor::new(payload)).map_err(Error::from)
    }

    pub fn is_potential_beacon(&self) -> bool {
        Self::parse_header(self.payload())
            .map(|header| header.mtype() == lorawan::MType::Proprietary)
            .unwrap_or(false)
    }

    pub fn to_rx1_pull_resp(&self, tx_power: u32) -> Result<pull_resp::TxPk> {
        self.inner_to_pull_resp(
            self.0.timestamp,
            self.0.frequency,
            self.0.datarate.parse()?,
            tx_power,
        )
    }

    pub fn to_rx2_pull_resp(&self, tx_power: u32) -> Result<Option<pull_resp::TxPk>> {
        let rx2 = match &self.0.rx2_window {
            Some(window) => window,
            None => return Ok(None),
        };

        self.inner_to_pull_resp(
            rx2.timestamp,
            rx2.frequency,
            rx2.datarate.parse()?,
            tx_power,
        )
        .map(Some)
    }

    fn inner_to_pull_resp(
        &self,
        timestamp: u64,
        frequency: f32,
        datarate: DataRate,
        tx_power: u32,
    ) -> Result<pull_resp::TxPk> {
        Ok(pull_resp::TxPk {
            time: Time::by_tmst(timestamp as u32),
            ipol: true,
            modu: Modulation::LORA,
            codr: CodingRate::_4_5,
            datr: datarate,
            // for normal lorawan packets we're not selecting different frequencies
            // like we are for PoC
            freq: frequency as f64,
            data: PhyData::new(self.0.payload.clone()),
            powe: tx_power as u64,
            rfch: 0,
            fdev: None,
            prea: None,
            ncrc: None,
        })
    }

    pub fn hash(&self) -> Vec<u8> {
        Sha256::digest(&self.0.payload).to_vec()
    }

    pub fn dc_payload(&self) -> u64 {
        const DC_PAYLOAD_SIZE: usize = 24;
        let payload_size = self.payload().len();
        if payload_size <= DC_PAYLOAD_SIZE {
            1
        } else {
            // integer div/ceil from: https://stackoverflow.com/a/2745086
            ((payload_size + DC_PAYLOAD_SIZE - 1) / DC_PAYLOAD_SIZE) as u64
        }
    }
}

fn to_hz(mhz: f32) -> u64 {
    (mhz * 1_000_000f32).trunc() as u64
}
