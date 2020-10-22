use crate::settings::Settings;
use anyhow::Result;
//use helium_proto::{packet::PacketType, Packet};
use semtech_udp::{
    //  pull_resp,
    server_runtime::{Event, UdpRuntime},
    //StringOrNum,
    Up as UdpPacket,
};
use std::{io, net::SocketAddr};
use tracing::{debug, info};

#[derive(Debug)]
pub struct Gateway {
    udp_runtime: UdpRuntime,
}

impl Gateway {
    pub async fn new(settings: &Settings) -> Result<Self> {
        let addr: SocketAddr = settings.semtech_udp.listen_addr.parse()?;
        let gateway = Gateway {
            udp_runtime: UdpRuntime::new(addr).await?,
        };
        Ok(gateway)
    }

    pub async fn run(&mut self, shutdown: triggered::Listener) -> Result<()> {
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
                                let packet = lorawan::PHYPayload::read(
                                    lorawan::Direction::Uplink,
                                    &mut io::Cursor::new(&base64::decode(
                                        received_packet.data.clone(),
                                    )?),
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
