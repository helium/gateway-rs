use crate::{base64, key, result::Result, router, settings::Settings};
use helium_proto::{packet::PacketType, Packet as LoraPacket, Region};
use log::{debug, info, warn};
use semtech_udp::{
    pull_resp, push_data,
    server_runtime::{Downlink, Event, UdpRuntime},
    StringOrNum, Up as UdpPacket,
};
use std::{sync::Arc, time::Duration};

pub const DOWNLINK_TIMEOUT_SECS: u64 = 5;
pub const UPLINK_TIMEOUT_SECS: u64 = 6;

#[derive(Debug)]
pub struct Gateway {
    settings: Settings,
    router: Arc<router::Client>,
    key: Arc<key::Key>,
    udp_runtime: UdpRuntime,
}

impl Gateway {
    pub async fn new(settings: &Settings) -> Result<Self> {
        let gateway = Gateway {
            settings: settings.clone(),
            router: Arc::new(router::Client::new(&settings)?),
            key: Arc::new(settings.key.clone()),
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
                        if let Some(rxpks) = &mut packet.data.rxpk {
                            // Sort packets by snr
                            // rxpk.sort_by(|a, b| b.get_snr().partial_cmp(&a.get_snr()).unwrap());
                            for rxpk in rxpks {
                                let router = self.router.clone();
                                let push_data = rxpk.clone();
                                let region = self.settings.region;
                                let key = self.key.clone();
                                let mut push_targets = vec![];
                                for target in &self.settings.routers {
                                    push_targets.push((
                                        target.clone(),
                                        self.udp_runtime.prepare_empty_downlink(packet.gateway_mac),
                                    ))
                                }
                                tokio::spawn(async move {
                                    handle_push_data(push_data, router, region, key, push_targets)
                                        .await
                                });
                            }
                        }
                    }
                    UdpPacket::PullData(_) => (), // Silently ignore since this happens often
                    _ => debug!("ignoring {:?}", packet),
                },
                Event::NoClientWithMac(_packet, mac) => {
                    warn!("Tried to send to client with unknown MAC: {:?}", mac)
                }
            }
        }
    }
}

async fn handle_push_data(
    push_data: push_data::RxPk,
    router: Arc<router::Client>,
    region: Region,
    key: Arc<key::Key>,
    push_targets: Vec<(reqwest::Url, Downlink)>,
) {
    let payload = match base64::decode_block(&push_data.get_data()) {
        Err(err) => return debug!("Ignoring bad push data: {:?}", err),
        Ok(v) => v,
    };
    if is_lonfi_packet(&payload) {
        info!("ignoring longfi packet")
    }
    // Handle lorawan packet, check if there is routing data
    match router::Routing::from_data(&payload) {
        Err(err) => return debug!("ignoring, bad routing data {:?}", err),
        Ok(None) => return debug!("ignoring, no routing data"),
        Ok(Some(routing)) => {
            // There's some routing data available. Construct the packet for the
            // router and make it into the signed message to post
            let router_packet = to_router_packet(&push_data, routing, &payload);
            let router_message = match router::Message::from_packet(router_packet, &key, region) {
                Err(err) => return debug!("unable to create router message: {:?}", err),
                Ok(m) => m,
            };
            for (router_address, mut downlink) in push_targets {
                let inner_router = router.clone();
                let inner_message = router_message.clone();
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
                                let pull_resp = to_pull_resp(&packet);
                                downlink.set_packet(pull_resp);
                                match downlink
                                    .dispatch(Some(Duration::from_secs(DOWNLINK_TIMEOUT_SECS)))
                                    .await
                                {
                                    Err(err) => debug!("Failed to send downlink {:?}", err),
                                    Ok(()) => (),
                                }
                            }
                        }
                        Err(err) => warn!("failed to route message: {:?}", err),
                    }
                });
            }
        }
    }
}

fn is_lonfi_packet(data: &[u8]) -> bool {
    let mut decoded = [0xFE, 65];
    longfi::Datagram::decode(data, &mut decoded).is_ok()
}

fn to_pull_resp(packet: &LoraPacket) -> pull_resp::TxPk {
    let payload = base64::encode_block(&packet.payload);
    let payload_len = payload.len();
    pull_resp::TxPk {
        imme: true,
        ipol: true,
        modu: "LORA".to_string(),
        codr: "4/5".to_string(),
        datr: packet.datarate.clone(),
        // for normal lorawan packets we're not selecting different frequencies
        // like we are for PoC
        freq: packet.frequency as f64,
        data: payload,
        size: payload_len as u64,
        powe: 27,
        rfch: 0,
        // tmst will be ignored since imme is true
        tmst: StringOrNum::N(packet.timestamp),
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
