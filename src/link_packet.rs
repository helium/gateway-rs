use crate::*;
use helium_crypto::Sign;
use helium_proto::{
    blockchain_state_channel_message_v1::Msg, packet::PacketType,
    routing_information::Data as RoutingData, BlockchainStateChannelMessageV1,
    BlockchainStateChannelPacketV1, BlockchainStateChannelResponseV1, Eui, Message as ProstMessage,
    Packet as LoraPacket, Region, RoutingInformation,
};
use semtech_udp::{pull_resp, push_data, CodingRate, MacAddress, Modulation, StringOrNum};

#[derive(Debug, Clone)]
pub struct LinkPacket {
    pub gateway_mac: MacAddress,
    pub packet: LoraPacket,
}

impl LinkPacket {
    pub fn from_push_data(push_data: &push_data::RxPk, gateway_mac: MacAddress) -> Result<Self> {
        let rssi = push_data
            .get_signal_rssi()
            .unwrap_or_else(|| push_data.get_channel_rssi());
        let packet = LoraPacket {
            r#type: PacketType::Lorawan.into(),
            signal_strength: rssi as f32,
            snr: push_data.get_snr() as f32,
            frequency: *push_data.get_frequency() as f32,
            timestamp: *push_data.get_timestamp(),
            datarate: push_data.get_datarate().to_string(),
            routing: mk_routing_information(push_data.get_data())?,
            payload: push_data.get_data().to_vec(),
            rx2_window: None,
            oui: 0,
        };
        Ok(Self {
            gateway_mac,
            packet,
        })
    }

    pub fn is_longfi(&self) -> bool {
        let mut decoded = [0xFE, 65];
        longfi::Datagram::decode(&self.packet.payload, &mut decoded).is_ok()
    }

    pub fn to_pull_resp(&self, use_rx2: bool) -> Result<Option<pull_resp::TxPk>> {
        let (timestamp, frequency, datarate) = if use_rx2 {
            if let Some(rx2) = &self.packet.rx2_window {
                (Some(rx2.timestamp), rx2.frequency, rx2.datarate.parse()?)
            } else {
                return Ok(None);
            }
        } else {
            (
                Some(self.packet.timestamp),
                self.packet.frequency,
                self.packet.datarate.parse()?,
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
            data: self.packet.payload.clone(),
            size: self.packet.payload.len() as u64,
            powe: 27,
            rfch: 0,
            tmst: match timestamp {
                Some(t) => StringOrNum::N(t),
                None => StringOrNum::S("immediate".to_string()),
            },
            tmms: None,
            fdev: None,
            prea: None,
            ncrc: None,
        }))
    }

    pub fn from_state_channel_message(
        message: BlockchainStateChannelMessageV1,
        gateway_mac: MacAddress,
    ) -> Option<Self> {
        match message {
            BlockchainStateChannelMessageV1 {
                msg:
                    Some(Msg::Response(BlockchainStateChannelResponseV1 {
                        downlink: Some(downlink),
                        ..
                    })),
            } => Some(Self {
                packet: downlink,
                gateway_mac,
            }),
            _ => None,
        }
    }

    pub fn to_state_channel_message(
        &self,
        keypair: &Keypair,
        region: Region,
    ) -> Result<BlockchainStateChannelMessageV1> {
        let mut router_packet = BlockchainStateChannelPacketV1 {
            packet: Some(self.packet.clone()),
            signature: vec![],
            hotspot: keypair.public_key.to_bytes().to_vec(),
            region: region.into(),
        };
        let mut encoded = vec![];
        router_packet.encode(&mut encoded)?;
        router_packet.signature = keypair.sign(&encoded)?;
        let message = BlockchainStateChannelMessageV1 {
            msg: Some(Msg::Packet(router_packet)),
        };
        Ok(message)
    }
}

pub fn mk_routing_information(payload: &[u8]) -> Result<Option<RoutingInformation>> {
    use lorawan::{Direction, PHYPayload, PHYPayloadFrame};
    use std::io::Cursor;
    match PHYPayload::read(Direction::Uplink, &mut Cursor::new(payload)) {
        Ok(packet) => {
            let routing_data = match &packet.payload {
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
        Err(err) => Err(err.into()),
    }
}
