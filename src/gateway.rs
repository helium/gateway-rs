use crate::{base64, key, result::Result, settings::Settings};
//use helium_proto::{packet::PacketType, Packet};
use log::{debug, info};
use semtech_udp::{
    //  pull_resp,
    server_runtime::{Event, UdpRuntime},
    //StringOrNum,
    Up as UdpPacket,
};
use std::net::SocketAddr;

#[derive(Debug)]
pub struct Gateway {
    listen_addr: SocketAddr,
    key: key::Key,
    udp_runtime: UdpRuntime,
}

impl Gateway {
    pub async fn new(settings: &Settings) -> Result<Self> {
        let listen_addr = settings.listen_addr()?;
        let gateway = Gateway {
            listen_addr,
            key: settings.key()?,
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
                            rxpk.sort_by(|a, b| b.snr().partial_cmp(&a.snr()).unwrap());
                            for received_packet in rxpk {
                                let mut packet_data =
                                    &base64::decode_block(&received_packet.data.clone())?[..];
                                let packet = lorawan::PHYPayload::read(
                                    lorawan::Direction::Uplink,
                                    &mut packet_data,
                                )?;
                                info!("Packet: {:?}", packet);
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
}

// fn to_helium_packet(packet: &RxPk) -> Result<Packet> {
//     let packet = Packet {
//         r#type: PacketType::Lorawan.into(),
//         signal_strength: packet.rssi() as f32,
//         snr: packet.snr() as f32,
//         frequency: packet.freq as f32,
//         timestamp: packet.tmst,
//         datarate: packet.datr.clone(),
//         payload: base64::decode(&packet.data)?,
//         routing: None,
//         rx2_window: None,
//         oui: 0,
//     };
// }
