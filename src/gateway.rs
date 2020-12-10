use crate::{error::Result, keypair, router, settings::Settings};
use helium_proto::{packet::PacketType, Packet as LoraPacket, Region};
use semtech_udp::{
    pull_resp, push_data,
    server_runtime::{Downlink, Error as SemtechError, Event, UdpRuntime},
    tx_ack, StringOrNum, Up as UdpPacket,
};
use slog::{debug, info, o, warn, Logger};
use std::{sync::Arc, time::Duration};
use tokio::sync::broadcast;

pub const DOWNLINK_TIMEOUT_SECS: u64 = 5;
pub const UPLINK_TIMEOUT_SECS: u64 = 6;

#[derive(Debug)]
pub struct Gateway {
    router: Arc<router::Client>,
    key: Arc<keypair::Keypair>,
    region: router::Region,
    router_targets: Vec<router::Url>,
    udp_runtime: UdpRuntime,
}

type Downlinks = (Downlink, Downlink);

impl Gateway {
    pub async fn new(settings: &Settings) -> Result<Self> {
        let gateway = Gateway {
            router_targets: settings.routers.clone(),
            region: settings.region,
            router: Arc::new(router::Client::new(&settings)?),
            key: settings.keypair.clone(),
            udp_runtime: UdpRuntime::new(settings.listen_addr).await?,
        };
        Ok(gateway)
    }

    pub async fn run(&mut self, shutdown: triggered::Listener, logger: &Logger) -> Result {
        let logger = logger.new(o!("module" => "gateway"));
        info!(logger, "starting gateway");
        loop {
            let logger = logger.clone();
            let event = tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                res = self.udp_runtime.recv() => match res {
                    Err(broadcast::RecvError::Closed) => return Err(broadcast::RecvError::Closed.into()),
                    Err(broadcast::RecvError::Lagged(skipped)) => {
                        warn!(logger.clone(), "skipped {} udp events", skipped);
                        continue;
                    },
                    Ok(v) => v,
                }
            };
            match event {
                Event::UnableToParseUdpFrame(buf) => {
                    info!(logger, "ignoring semtech udp parsing error for {:?}", buf);
                }
                Event::NewClient((mac, addr)) => {
                    info!(logger, "new packet forwarder client: {}, {}", mac, addr);
                }
                Event::UpdateClient((mac, addr)) => {
                    info!(logger, "mac existed, but IP updated: {}, {}", mac, addr);
                }
                Event::Packet(packet) => match packet {
                    UdpPacket::PushData(mut packet) => {
                        if let Some(rxpks) = &mut packet.data.rxpk {
                            // Sort packets by snr
                            // rxpk.sort_by(|a, b| b.get_snr().partial_cmp(&a.get_snr()).unwrap());
                            for rxpk in rxpks {
                                let router = self.router.clone();
                                let push_data = rxpk.clone();
                                let region = self.region;
                                let key = self.key.clone();
                                let mut push_targets = vec![];
                                let logger = logger.clone();
                                for target in &self.router_targets {
                                    let downlinks = (
                                        // first downlink
                                        self.udp_runtime.prepare_empty_downlink(packet.gateway_mac),
                                        // 2nd downlink window if requested by the router response
                                        self.udp_runtime.prepare_empty_downlink(packet.gateway_mac),
                                    );

                                    push_targets.push((target.clone(), downlinks))
                                }
                                tokio::spawn(async move {
                                    handle_push_data(
                                        push_data,
                                        router,
                                        region,
                                        key,
                                        push_targets,
                                        logger,
                                    )
                                    .await
                                });
                            }
                        }
                    }
                    _ => debug!(logger, "ignoring {:?}", packet),
                },
                Event::NoClientWithMac(_packet, mac) => {
                    warn!(logger, "send to client with unknown MAC: {:?}", mac)
                }
            }
        }
    }
}

async fn handle_push_data(
    push_data: push_data::RxPk,
    router: Arc<router::Client>,
    region: Region,
    key: Arc<keypair::Keypair>,
    push_targets: Vec<(reqwest::Url, Downlinks)>,
    logger: Logger,
) {
    let payload = match base64::decode(&push_data.get_data()) {
        Err(err) => return debug!(logger, "ignoring bad push data: {:?}", err),
        Ok(v) => v,
    };
    if is_lonfi_packet(&payload) {
        info!(logger, "ignoring longfi packet")
    }
    // Handle lorawan packet, check if there is routing data
    match router::Routing::from_data(&payload) {
        Err(err) => return debug!(logger, "ignoring, bad routing data {:?}", err),
        Ok(None) => return debug!(logger, "ignoring, no routing data"),
        Ok(Some(routing)) => {
            // There's some routing data available. Construct the packet for the
            // router and make it into the signed message to post
            let router_packet = to_router_packet(&push_data, routing, &payload);
            let router_message = match router::Message::from_packet(router_packet, &key, region) {
                Err(err) => return warn!(logger, "unable to create router message: {:?}", err),
                Ok(m) => m,
            };
            for (router_address, downlinks) in push_targets {
                let inner_router = router.clone();
                let inner_message = router_message.clone();
                let logger = logger.clone();
                // Spawn of each target router target into its own thread.
                tokio::spawn(async move {
                    // Send the message
                    match inner_router
                        .send(
                            &router_address,
                            &inner_message,
                            Duration::from_secs(UPLINK_TIMEOUT_SECS),
                        )
                        .await
                    {
                        // No response, we're done
                        Ok(None) => (),
                        // A response, dispatch to the device if there's a downlink
                        Ok(Some(response)) => {
                            if let Some(packet) = response.downlink() {
                                match send_downlink(packet, downlinks, logger.clone()).await {
                                    Ok(()) => debug!(logger, "downlink sent"),
                                    Err(err) => warn!(logger, "failed to send downlink: {:?}", err),
                                }
                            }
                        }
                        Err(err) => warn!(logger, "failed to route message: {:?}", err),
                    }
                });
            }
        }
    }
}

async fn send_downlink(
    packet: &LoraPacket,
    downlinks: Downlinks,
    logger: Logger,
) -> std::result::Result<(), SemtechError> {
    let (mut downlink, mut downlink_rx2) = downlinks;
    let pull_resp = mk_pull_resp(
        &packet.payload,
        packet.frequency,
        packet.datarate.clone(),
        Some(packet.timestamp),
    );
    downlink.set_packet(pull_resp);
    debug!(logger, "sending downlink {:?}", packet);
    match downlink
        .dispatch(Some(Duration::from_secs(DOWNLINK_TIMEOUT_SECS)))
        .await
    {
        // On a too early or too late error retry on the rx2 slot if available.
        Err(SemtechError::AckError(tx_ack::Error::TOO_EARLY))
        | Err(SemtechError::AckError(tx_ack::Error::TOO_LATE)) => {
            if let Some(rx2) = &packet.rx2_window {
                let pull_resp = mk_pull_resp(
                    &packet.payload,
                    rx2.frequency,
                    rx2.datarate.clone(),
                    Some(rx2.timestamp),
                );
                downlink_rx2.set_packet(pull_resp);
                debug!(logger, "sending rx2 downlink {:?}", packet);
                downlink_rx2
                    .dispatch(Some(Duration::from_secs(DOWNLINK_TIMEOUT_SECS)))
                    .await
            } else {
                Ok(())
            }
        }
        Err(SemtechError::AckError(tx_ack::Error::NONE)) => Ok(()),
        other => other,
    }
}

fn is_lonfi_packet(data: &[u8]) -> bool {
    let mut decoded = [0xFE, 65];
    longfi::Datagram::decode(data, &mut decoded).is_ok()
}

fn mk_pull_resp(
    data: &[u8],
    frequency: f32,
    datarate: String,
    timestamp: Option<u64>,
) -> pull_resp::TxPk {
    pull_resp::TxPk {
        imme: timestamp.is_none(),
        ipol: true,
        modu: "LORA".to_string(),
        codr: "4/5".to_string(),
        datr: datarate,
        // for normal lorawan packets we're not selecting different frequencies
        // like we are for PoC
        freq: frequency as f64,
        data: base64::encode(data),
        size: data.len() as u64,
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
    }
}

fn to_router_packet(
    push_data: &push_data::RxPk,
    routing: router::Routing,
    payload: &[u8],
) -> LoraPacket {
    LoraPacket {
        r#type: PacketType::Lorawan.into(),
        signal_strength: push_data.get_rssi() as f32,
        snr: push_data.get_snr() as f32,
        frequency: *push_data.get_frequency() as f32,
        timestamp: *push_data.get_timestamp(),
        datarate: push_data.get_datarate(),
        payload: payload.to_vec(),
        routing: Some(routing.into()),
        rx2_window: None,
        oui: 0,
    }
}

// fn mk_identity(settings: &Settings) -> Result<reqwest::Identity> {
//     use openssl::{
//         asn1::Asn1Time,
//         hash::MessageDigest,
//         nid::Nid,
//         pkcs12::Pkcs12,
//         x509::{extension::KeyUsage, X509Name, X509},
//     };

//     let subject_name = settings.key.to_string();
//     let pkey = &settings.key.0;
//     let mut name = X509Name::builder()?;
//     name.append_entry_by_nid(Nid::COMMONNAME, &subject_name)?;
//     let name = name.build();
//     let key_usage = KeyUsage::new().digital_signature().build()?;

//     let mut builder = X509::builder()?;
//     builder.set_version(2)?;
//     builder.set_not_before(&Asn1Time::days_from_now(0)?.as_ref())?;
//     builder.set_not_after(&Asn1Time::days_from_now(4096)?.as_ref())?;
//     builder.set_subject_name(&name)?;
//     builder.set_issuer_name(&name)?;
//     builder.append_extension(key_usage)?;
//     builder.set_pubkey(&pkey)?;
//     unsafe {
//         // null digest required for ed25519
//         builder.sign(pkey, MessageDigest::from_ptr(std::ptr::null()))?;
//     };
//     let cert = builder.build();

//     const PKCS12_PASSWD: &str = "no_password";
//     let pkcs12_builder = Pkcs12::builder();
//     let pkcs12 = pkcs12_builder.build(PKCS12_PASSWD, &subject_name, &pkey, &cert)?;
//     let der = pkcs12.to_der()?;
//     let identity = reqwest::Identity::from_pkcs12_der(&der, PKCS12_PASSWD)?;
//     Ok(identity)
// }
