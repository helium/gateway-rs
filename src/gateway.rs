use crate::{base64, key, result::Result, settings::Settings};
use helium_proto::{
    packet::PacketType, routing_information::Data as RoutingData, Eui, Packet as HeliumPacket,
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
use std::{io::Cursor, net::SocketAddr, sync::Arc};

#[derive(Debug)]
pub struct Gateway {
    listen_addr: SocketAddr,
    key: Arc<key::Key>,
    udp_runtime: UdpRuntime,
}

impl Gateway {
    pub async fn new(settings: &Settings) -> Result<Self> {
        let listen_addr = settings.listen_addr;
        let gateway = Gateway {
            listen_addr,
            key: settings.key.clone(),
            udp_runtime: UdpRuntime::new(listen_addr).await?,
        };
        Ok(gateway)
    }

    pub async fn run(&mut self, shutdown: triggered::Listener) -> Result {
        info!(
            "Starting gateway listener {} on {}",
            self.key, self.listen_addr
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
                    info!("Semtech UDP Parsing Error");
                    info!("UDP data: {:?}", buf);
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
                            debug!("Received packets:");
                            // Sort packets by snr
                            rxpk.sort_by(|a, b| b.snr().partial_cmp(&a.snr()).unwrap());
                            for received_packet in rxpk {
                                let packet_data =
                                    &base64::decode_block(&received_packet.data.clone())?[..];
                                if self.route_longfi_data(&received_packet, &packet_data).await {
                                    self.route_lorawan_data(&received_packet, &packet_data)
                                        .await;
                                }
                            }
                        }
                    }
                    _ => debug!("{:?}", packet),
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

    async fn route_lorawan_data(&self, received_packet: &push_data::RxPk, data: &[u8]) -> bool {
        let mut cursor = Cursor::new(data);
        match lorawan::PHYPayload::read(lorawan::Direction::Uplink, &mut cursor) {
            Ok(packet) => match mk_routing_information(&packet) {
                // Ignore packets with no available routing information
                None => false,
                Some(routing) => {
                    let helium_packet = mk_helium_packet(received_packet, Some(routing), data);
                    info!("TO_ROUTER: {:?}", helium_packet);
                    true
                }
            },
            // invalid lorawan packet
            Err(_) => false,
        }
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

fn mk_helium_packet(
    packet: &push_data::RxPk,
    routing: Option<RoutingInformation>,
    data: &[u8],
) -> HeliumPacket {
    HeliumPacket {
        r#type: PacketType::Lorawan.into(),
        signal_strength: packet.rssi() as f32,
        snr: packet.snr() as f32,
        frequency: packet.freq as f32,
        timestamp: packet.tmst,
        datarate: packet.datr.clone(),
        payload: data.to_vec(),
        routing,
        rx2_window: None,
        oui: 0,
    }
}
