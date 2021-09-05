use crate::*;
use helium_proto::{
    blockchain_state_channel_message_v1::Msg, packet::PacketType,
    routing_information::Data as RoutingData, BlockchainStateChannelMessageV1,
    BlockchainStateChannelOfferV1, BlockchainStateChannelPacketV1,
    BlockchainStateChannelResponseV1, Eui, Region, RoutingInformation,
};
use lorawan::PHYPayloadFrame;
use semtech_udp::{pull_resp, push_data, CodingRate, MacAddress, Modulation, StringOrNum};
use sha2::{Digest, Sha256};
use std::ops::Deref;

#[derive(Debug, Clone)]
pub struct Packet(helium_proto::Packet);

impl Deref for Packet {
    type Target = helium_proto::Packet;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct LinkPacket {
    pub gateway_mac: MacAddress,
    pub packet: Packet,
}

impl LinkPacket {
    pub fn from_push_data(push_data: &push_data::RxPk, gateway_mac: MacAddress) -> Result<Self> {
        let rssi = push_data
            .get_signal_rssi()
            .unwrap_or_else(|| push_data.get_channel_rssi());
        let packet = Packet(helium_proto::Packet {
            r#type: PacketType::Lorawan.into(),
            signal_strength: rssi as f32,
            snr: push_data.get_snr() as f32,
            frequency: *push_data.get_frequency() as f32,
            timestamp: *push_data.get_timestamp() as u64,
            datarate: push_data.get_datarate().to_string(),
            routing: routing_information(&parse_frame(
                lorawan::Direction::Uplink,
                push_data.get_data(),
            )?)?,
            payload: push_data.get_data().to_vec(),
            rx2_window: None,
            oui: 0,
        });
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
            if let Some(rx2) = &self.packet.0.rx2_window {
                (Some(rx2.timestamp), rx2.frequency, rx2.datarate.parse()?)
            } else {
                return Ok(None);
            }
        } else {
            (
                Some(self.packet.0.timestamp),
                self.packet.0.frequency,
                self.packet.0.datarate.parse()?,
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
            data: self.packet.0.payload.clone(),
            size: self.packet.0.payload.len() as u64,
            powe: 27,
            rfch: 0,
            tmst: match timestamp {
                Some(t) => StringOrNum::N(t as u32),
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
                packet: Packet(downlink),
                gateway_mac,
            }),
            _ => None,
        }
    }

    pub fn to_state_channel_packet(
        &self,
        keypair: &Keypair,
        region: Region,
    ) -> Result<StateChannelMessage> {
        let mut packet = BlockchainStateChannelPacketV1 {
            packet: Some(self.packet.0.clone()),
            signature: vec![],
            hotspot: keypair.public_key().into(),
            region: region.into(),
            hold_time: 0,
        };
        packet.signature = packet.sign(keypair)?;
        Ok(StateChannelMessage::from(packet))
    }

    pub fn to_state_channel_offer(
        &self,
        keypair: &Keypair,
        region: Region,
    ) -> Result<StateChannelMessage> {
        let frame = parse_frame(lorawan::Direction::Uplink, &self.packet.0.payload)?;
        let mut offer = BlockchainStateChannelOfferV1 {
            packet_hash: self.packet_hash(),
            payload_size: self.packet.0.payload.len() as u64,
            fcnt: frame.fcnt().unwrap_or(0) as u32,
            hotspot: keypair.public_key().into(),
            region: region.into(),
            routing: routing_information(&frame)?,
            signature: vec![],
        };
        offer.signature = offer.sign(keypair)?;
        Ok(StateChannelMessage::from(offer))
    }

    fn packet_hash(&self) -> Vec<u8> {
        Sha256::digest(&self.packet.0.payload).to_vec()
    }
}

fn parse_frame(direction: lorawan::Direction, payload: &[u8]) -> Result<PHYPayloadFrame> {
    use std::io::Cursor;
    lorawan::PHYPayload::read(direction, &mut Cursor::new(payload))
        .map(|p| p.payload)
        .map_err(Error::from)
}

fn routing_information(frame: &PHYPayloadFrame) -> Result<Option<RoutingInformation>> {
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
