use crate::settings::Settings;
use anyhow::Result;
use semtech_udp::{
    pull_resp,
    server_runtime::{Event, UdpRuntime},
    StringOrNum, Up as Packet,
};
use std::net::SocketAddr;
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
                    Packet::PushData(packet) => {
                        if let Some(rxpk) = &packet.data.rxpk {
                            debug!("Received packets:");
                            for received_packet in rxpk {
                                println!("\t{:?}", received_packet);

                                let buffer = [1, 2, 3, 4];
                                let size = buffer.len() as u64;
                                let data = base64::encode(buffer);
                                let tmst = StringOrNum::N(received_packet.tmst + 1_000_000);

                                let txpk = pull_resp::TxPk {
                                    imme: false,
                                    tmst,
                                    freq: 902.800_000,
                                    rfch: 0,
                                    powe: 27,
                                    modu: "LORA".to_string(),
                                    datr: "SF8BW500".to_string(),
                                    codr: "4/5".to_string(),
                                    ipol: true,
                                    size,
                                    data,
                                    tmms: None,
                                    fdev: None,
                                    prea: None,
                                    ncrc: None,
                                };

                                let prepared_send =
                                    self.udp_runtime.prepare_send(txpk, packet.gateway_mac);

                                tokio::spawn(async move {
                                    if let Err(e) = prepared_send.dispatch().await {
                                        panic!("Transmit Dispatch threw error: {:?}", e)
                                    } else {
                                        debug!("Send complete");
                                    }
                                });
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
