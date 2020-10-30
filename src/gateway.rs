use crate::{base64, result::Result, settings::Settings};
use helium_proto::{
    packet::PacketType, routing_information::Data as RoutingData,
    BlockchainStateChannelPacketV1 as RouterPacket, Eui, Message, Packet as HeliumPacket,
    RoutingInformation,
};
use log::{debug, info};
use lorawan::{PHYPayload, PHYPayloadFrame};
use semtech_udp::{
    //  pull_resp,
    push_data,
    server_runtime::{Event, UdpRuntime},
    //StringOrNum,
    Up as UdpPacket,
};
use std::io::Cursor;

#[derive(Debug)]
pub struct Gateway {
    settings: Settings,
    udp_runtime: UdpRuntime,
}

impl Gateway {
    pub async fn new(settings: &Settings) -> Result<Self> {
        let gateway = Gateway {
            settings: settings.clone(),
            udp_runtime: UdpRuntime::new(settings.listen_addr).await?,
        };
        Ok(gateway)
    }

    pub async fn run(&mut self, shutdown: triggered::Listener) -> Result {
        info!(
            "Starting gateway listener {} on {}",
            self.settings.key, self.settings.listen_addr
        );
        loop {
            let event = tokio::select! {
                _ = shutdown.clone() => {
                    info!("Shutting down gateway");
                    return Ok(())
                },
                res = self.udp_runtime.recv() => res?
            };

            match event {
                Event::UnableToParseUdpFrame(buf) => {
                    info!("Ignorint semtech udp parsing error for {:?}", buf);
                }
                Event::NewClient((mac, addr)) => {
                    info!("New packet forwarder client: {}, {}", mac, addr);
                }
                Event::UpdateClient((mac, addr)) => {
                    info!("Mac existed, but IP updated: {}, {}", mac, addr);
                }
                Event::Packet(packet) => match packet {
                    UdpPacket::PushData(mut packet) => {
                        if let Some(rxpk) = &mut packet.data.rxpk {
                            // Sort packets by snr
                            rxpk.sort_by(|a, b| b.get_snr().partial_cmp(&a.get_snr()).unwrap());
                            for received_packet in rxpk {
                                let packet_data =
                                    &base64::decode_block(&received_packet.get_data().clone())?[..];
                                if !self.route_longfi_data(&received_packet, &packet_data).await {
                                    self.route_lorawan_data(&received_packet, &packet_data)
                                        .await;
                                }
                            }
                        }
                    }
                    _ => debug!("ignoring {:?}", packet),
                },
                Event::NoClientWithMac(_packet, mac) => {
                    info!("Tried to send to client with unknown MAC: {:?}", mac)
                }
            }
        }
    }

    async fn route_longfi_data(&self, _received_packet: &push_data::RxPk, data: &[u8]) -> bool {
        let mut decoded = [0xFE, 65];
        match longfi::Datagram::decode(data, &mut decoded) {
            Ok(_) => {
                info!("Decoded longfi packet, ignoring");
                true
            }
            Err(_) => false,
        }
    }

    async fn route_lorawan_data(&self, received_packet: &push_data::RxPk, data: &[u8]) {
        let mut cursor = Cursor::new(data);
        match lorawan::PHYPayload::read(lorawan::Direction::Uplink, &mut cursor) {
            Ok(packet) => match mk_routing_information(&packet) {
                // Ignore packets with no available routing information
                None => {
                    debug!("No routing info for: {:?}", packet);
                }
                Some(routing) => {
                    match self.to_router_packet(received_packet, Some(routing), data) {
                        Ok(router_packet) => self.send_to_router(&router_packet).await,
                        Err(err) => info!("Failed to construct router packet: {:?}", err),
                    }
                }
            },
            // invalid lorawan packet
            Err(err) => {
                info!("invalid lorawan packet {:?}", err);
            }
        }
    }

    async fn send_to_router(&self, packet: &RouterPacket) {
        debug!("SEND TO ROUTER: {:?}", packet);
    }

    fn to_router_packet(
        &self,
        packet: &push_data::RxPk,
        routing: Option<RoutingInformation>,
        data: &[u8],
    ) -> Result<RouterPacket> {
        let helium_packet = HeliumPacket {
            r#type: PacketType::Lorawan.into(),
            signal_strength: packet.get_rssi() as f32,
            snr: packet.get_snr() as f32,
            frequency: *packet.get_frequency() as f32,
            timestamp: *packet.get_timestamp(),
            datarate: packet.get_datarate(),
            payload: data.to_vec(),
            routing,
            rx2_window: None,
            oui: 0,
        };
        let mut envelope = RouterPacket {
            packet: Some(helium_packet),
            signature: vec![],
            hotspot: self.settings.key.to_key_bin(),
            region: self.settings.region.into(),
        };
        let mut encoded = vec![];
        envelope.encode(&mut encoded)?;
        envelope.signature = self.settings.key.sign(&encoded)?;
        Ok(envelope)
    }
}

fn mk_routing_information(packet: &PHYPayload) -> Option<RoutingInformation> {
    let routing_data = match &packet.payload {
        PHYPayloadFrame::JoinRequest(request) => Some(RoutingData::Eui(Eui {
            deveui: request.dev_eui,
            appeui: request.app_eui,
        })),
        PHYPayloadFrame::MACPayload(mac_payload) => {
            Some(RoutingData::Devaddr(mac_payload.dev_addr()))
        }
        _ => None,
    };
    routing_data.map(|r| RoutingInformation { data: Some(r) })
}
