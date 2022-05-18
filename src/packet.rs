use crate::{error::DecodeError, Error, Result};
use helium_proto::{
    packet::PacketType, routing_information::Data as RoutingData, BlockchainStateChannelResponseV1,
    Eui, RoutingInformation,
};
use lorawan::PHYPayloadFrame;
use semtech_udp::{
    pull_resp,
    push_data::{self, CRC},
    CodingRate, DataRate, Modulation, StringOrNum,
};
use sha2::{Digest, Sha256};
use std::{convert::TryFrom, fmt, ops::Deref, str::FromStr};

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
                snr: rxpk.get_snr() as f32,
                frequency: *rxpk.get_frequency() as f32,
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

impl From<helium_proto::Packet> for Packet {
    fn from(v: helium_proto::Packet) -> Self {
        Self(v)
    }
}

impl Packet {
    pub fn routing(&self) -> &Option<RoutingInformation> {
        &self.0.routing
    }

    pub fn is_longfi(&self) -> bool {
        let mut decoded = [0xFE, 65];
        longfi::Datagram::decode(&self.0.payload, &mut decoded).is_ok()
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

    pub fn to_pull_resp(&self, use_rx2: bool) -> Result<Option<pull_resp::TxPk>> {
        let (timestamp, frequency, datarate) = if use_rx2 {
            if let Some(rx2) = &self.0.rx2_window {
                (Some(rx2.timestamp), rx2.frequency, rx2.datarate.parse()?)
            } else {
                return Ok(None);
            }
        } else {
            (
                Some(self.0.timestamp),
                self.0.frequency,
                self.0.datarate.parse()?,
            )
        };
        Ok(Some(pull_resp::TxPk {
            imme: timestamp.is_none(),
            ipol: true,
            modu: Modulation::LORA,
            codr: CodingRate::_4_5,
            datr: datarate,
            // for normal lorawan packets we're not selecting different frequencies
            // like we are for PoC
            freq: frequency as f64,
            data: self.0.payload.clone(),
            size: self.0.payload.len() as u64,
            powe: 27,
            rfch: 0,
            tmst: match timestamp {
                Some(t) => Some(StringOrNum::N(t as u32)),
                None => Some(StringOrNum::S("immediate".to_string())),
            },
            tmms: None,
            fdev: None,
            prea: None,
            ncrc: None,
        }))
    }

    pub fn from_state_channel_response(response: BlockchainStateChannelResponseV1) -> Option<Self> {
        response.downlink.map(Self)
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
