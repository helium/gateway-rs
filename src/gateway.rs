use crate::{
    beaconer, packet, packet_router, region_watcher, sync, DecodeError, Error, PacketDown,
    PacketUp, PublicKey, RegionParams, Result, Settings,
};
use beacon::Beacon;
use lorawan::PHYPayload;
use semtech_udp::{
    pull_resp::{self, Time},
    server_runtime::{Error as SemtechError, Event, UdpRuntime},
    tx_ack,
    tx_ack::Error as TxAckErr,
    CodingRate, MacAddress, Modulation,
};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub const DOWNLINK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct BeaconResp {
    pub powe: i32,
    pub tmst: u32,
}

#[derive(Debug)]
pub enum Message {
    Downlink(PacketDown),
    TransmitBeacon(Beacon, sync::ResponseSender<Result<BeaconResp>>),
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("unknown beacon tx power")]
    NoBeaconTxPower,
    #[error("beacon transmit failed")]
    BeaconTxFailure,
}

pub type MessageSender = sync::MessageSender<Message>;
pub type MessageReceiver = sync::MessageReceiver<Message>;

pub fn message_channel() -> (MessageSender, MessageReceiver) {
    sync::message_channel(10)
}

impl MessageSender {
    pub async fn downlink(&self, packet: PacketDown) {
        self.send(Message::Downlink(packet)).await
    }

    /// Send a non-inverted (`ipol = false`) beacon packet that is receivable by
    /// other gateways.
    ///
    /// Essentially, this packet looks like a regular uplink packet to other
    /// gateways until further inspection.
    pub async fn transmit_beacon(&self, beacon: Beacon) -> Result<BeaconResp> {
        self.request(move |tx| Message::TransmitBeacon(beacon, tx))
            .await?
    }
}

pub struct Gateway {
    public_key: PublicKey,
    messages: MessageReceiver,
    uplinks: packet_router::MessageSender,
    beacons: beaconer::MessageSender,
    downlink_mac: MacAddress,
    udp_runtime: UdpRuntime,
    listen_address: String,
    region_watch: region_watcher::MessageReceiver,
    region_params: RegionParams,
}

impl Gateway {
    pub async fn new(
        settings: &Settings,
        messages: MessageReceiver,
        region_watch: region_watcher::MessageReceiver,
        uplinks: packet_router::MessageSender,
        beacons: beaconer::MessageSender,
    ) -> Result<Self> {
        let region_params = region_watcher::current_value(&region_watch);
        let public_key = settings.keypair.public_key().clone();
        let gateway = Gateway {
            public_key,
            messages,
            uplinks,
            beacons,
            downlink_mac: Default::default(),
            listen_address: settings.listen.clone(),
            udp_runtime: UdpRuntime::new(&settings.listen).await.map_err(Box::new)?,
            region_watch,
            region_params,
        };
        Ok(gateway)
    }

    pub async fn run(&mut self, shutdown: &triggered::Listener) -> Result {
        info!(listen = &self.listen_address, "starting");
        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!( "shutting down");
                    return Ok(())
                },
                event = self.udp_runtime.recv() =>
                    self.handle_udp_event(event).await?,
                message = self.messages.recv() => match message {
                    Some(message) => self.handle_message(message).await,
                    None => {
                        warn!("ignoring closed message channel");
                        continue;
                    }
                },
                region_change = self.region_watch.changed() => match region_change {
                    Ok(()) => {
                        let new_region_params = region_watcher::current_value(&self.region_watch);
                        // Only log if region parameters have changed to avoid
                        // log noise
                        if self.region_params != new_region_params {
                            info!(region = RegionParams::to_string(&new_region_params), "region updated");
                        }
                        self.region_params = new_region_params;
                    }
                    Err(_) => warn!("region watch disconnected")
                },
            }
        }
    }

    async fn handle_udp_event(&mut self, event: Event) -> Result {
        match event {
            Event::UnableToParseUdpFrame(e, buf) => {
                warn!(raw_bytes = ?buf, "ignoring semtech udp parsing error {e}");
            }
            Event::NewClient((mac, addr)) => {
                info!(%mac, %addr, "new packet forwarder client");
                self.downlink_mac = mac;
            }
            Event::UpdateClient((mac, addr)) => {
                info!(%mac, %addr, "mac existed, but IP updated")
            }
            Event::ClientDisconnected((mac, addr)) => {
                info!(%mac, %addr, "disconnected packet forwarder")
            }
            Event::PacketReceived(rxpk, _gateway_mac) => {
                match PacketUp::from_rxpk(rxpk, &self.public_key, self.region_params.region) {
                    Ok(packet) if packet.is_potential_beacon() => {
                        self.handle_potential_beacon(packet).await;
                    }
                    Ok(packet) if packet.is_uplink() => {
                        self.handle_uplink(packet, Instant::now()).await
                    }
                    Ok(packet) => {
                        info!(%packet, "ignoring non-uplink packet");
                    }
                    Err(Error::Decode(DecodeError::CrcDisabled)) => {
                        debug!("ignoring packet with disabled crc");
                    }
                    Err(Error::Decode(DecodeError::InvalidDataRate(datarate))) => {
                        debug!(%datarate, "ignoring packet with invalid datarate");
                    }
                    Err(err) => {
                        warn!(%err, "ignoring push_data");
                    }
                }
            }
            Event::NoClientWithMac(_packet, mac) => {
                info!(%mac, "ignoring send to client with unknown MAC")
            }
            Event::StatReceived(stat, mac) => {
                debug!(%mac, ?stat, "received stat")
            }
        };
        Ok(())
    }

    async fn handle_potential_beacon(&mut self, packet: PacketUp) {
        if self.region_params.is_unknown() {
            info!(downlink_mac = %self.downlink_mac, uplink = %packet, "ignored potential beacon, no region");
            return;
        }
        info!(downlink_mac = %self.downlink_mac, uplink = %packet, "received potential beacon");
        self.beacons.received_beacon(packet).await
    }

    async fn handle_uplink(&mut self, packet: PacketUp, received: Instant) {
        if self.region_params.is_unknown() {
            info!(
                downlink_mac = %self.downlink_mac,
                uplink = %packet,
                region = %self.region_params,
                "ignored uplink");
            return;
        }
        info!(
            downlink_mac = %self.downlink_mac,
            uplink = %packet,
            region = %self.region_params,
            "received uplink");
        self.uplinks.uplink(packet, received).await;
    }

    async fn handle_message(&mut self, message: Message) {
        match message {
            Message::Downlink(packet) => self.handle_downlink(packet).await,
            Message::TransmitBeacon(beacon, tx_resp) => {
                self.handle_transmit_beacon(beacon, tx_resp).await
            }
        }
    }

    fn max_tx_power(&mut self) -> Result<u32> {
        Ok(self.region_params.max_conducted_power()?)
    }

    async fn handle_transmit_beacon(
        &mut self,
        beacon: Beacon,
        responder: sync::ResponseSender<Result<BeaconResp>>,
    ) {
        let tx_power = match self.max_tx_power() {
            Ok(tx_power) => tx_power,
            Err(err) => {
                warn!(%err, "beacon transmit");
                responder.send(Err(err));
                return;
            }
        };

        let packet = match beacon_to_pull_resp(&beacon, tx_power as u64) {
            Ok(packet) => packet,
            Err(err) => {
                warn!(%err, "failed to construct beacon pull resp");
                responder.send(Err(err));
                return;
            }
        };

        let beacon_tx = self.udp_runtime.prepare_downlink(packet, self.downlink_mac);

        tokio::spawn(async move {
            let beacon_id = beacon.beacon_id();
            match beacon_tx.dispatch(Some(DOWNLINK_TIMEOUT)).await {
                Ok(tmst) => {
                    info!(
                        beacon_id,
                        %tx_power,
                        ?tmst,
                        "beacon transmitted"
                    );
                    responder.send(Ok(BeaconResp {
                        powe: tx_power as i32,
                        tmst: tmst.unwrap_or(0),
                    }));
                    tmst
                }
                Err(err) => {
                    if let semtech_udp::server_runtime::Error::Ack(
                        tx_ack::Error::AdjustedTransmitPower(power_used, tmst),
                    ) = err
                    {
                        match power_used {
                            None => {
                                warn!("packet transmitted with adjusted power, but packet forwarder does not indicate power used.");
                                responder.send(Err(GatewayError::NoBeaconTxPower.into()));
                            }
                            Some(actual_power) => {
                                info!(
                                    beacon_id,
                                    actual_power,
                                    ?tmst,
                                    "beacon transmitted with adjusted power output",
                                );
                                responder.send(Ok(BeaconResp {
                                    powe: actual_power,
                                    tmst: tmst.unwrap_or(0),
                                }));
                            }
                        }
                        tmst
                    } else {
                        warn!(beacon_id, %err, "failed to transmit beacon");
                        responder.send(Err(GatewayError::BeaconTxFailure.into()));
                        None
                    }
                }
            }
        });
    }

    async fn handle_downlink(&mut self, downlink: PacketDown) {
        let tx_power = match self.max_tx_power() {
            Ok(tx_power) => tx_power,
            Err(err) => {
                warn!(%err, "downlink transmit");
                return;
            }
        };

        let (mut downlink_rx1, mut downlink_rx2) = (
            // first downlink
            self.udp_runtime.prepare_empty_downlink(self.downlink_mac),
            // 2nd downlink window if requested by the router response
            self.udp_runtime.prepare_empty_downlink(self.downlink_mac),
        );

        let downlink_mac = self.downlink_mac;

        tokio::spawn(async move {
            if let Ok(txpk) = downlink.to_rx1_pull_resp(tx_power) {
                info!(%downlink_mac, "rx1 downlink {txpk}",);

                downlink_rx1.set_packet(txpk);
                match downlink_rx1.dispatch(Some(DOWNLINK_TIMEOUT)).await {
                    // On a too early or too late error retry on the rx2 slot if available.
                    Err(SemtechError::Ack(TxAckErr::TooEarly | TxAckErr::TooLate)) => {
                        if let Ok(Some(txpk)) = downlink.to_rx2_pull_resp(tx_power) {
                            info!(%downlink_mac, "rx2 downlink {txpk}");

                            downlink_rx2.set_packet(txpk);
                            match downlink_rx2.dispatch(Some(DOWNLINK_TIMEOUT)).await {
                                Err(SemtechError::Ack(TxAckErr::AdjustedTransmitPower(_, _))) => {
                                    warn!("rx2 downlink sent with adjusted transmit power");
                                }
                                Err(err) => warn!(%err, "ignoring rx2 downlink error"),
                                _ => (),
                            }
                        }
                    }
                    Err(SemtechError::Ack(TxAckErr::AdjustedTransmitPower(_, _))) => {
                        warn!("rx1 downlink sent with adjusted transmit power");
                    }
                    Err(err) => {
                        warn!(%err, "ignoring rx1 downlink error");
                    }
                    Ok(_) => (),
                }
            }
        });
    }
}

pub fn beacon_to_pull_resp(beacon: &Beacon, tx_power: u64) -> Result<pull_resp::TxPk> {
    let datr = packet::datarate::from_proto(beacon.datarate)?;
    let freq = packet::to_mhz(beacon.frequency as f64);
    let data: Vec<u8> = PHYPayload::proprietary(beacon.data.as_slice()).try_into()?;

    Ok(pull_resp::TxPk {
        time: Time::immediate(),
        ipol: false,
        modu: Modulation::LORA,
        codr: Some(CodingRate::_4_5),
        datr,
        freq,
        data: pull_resp::PhyData::new(data),
        powe: tx_power,
        rfch: 0,
        fdev: None,
        prea: None,
        ncrc: None,
    })
}
