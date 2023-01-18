use crate::{
    beaconer, error::RegionError, region_watcher, router, sync, Packet, RegionParams, Result,
    Settings,
};
use beacon::Beacon;
use lorawan::PHYPayload;
use semtech_udp::{
    pull_resp,
    server_runtime::{Error as SemtechError, Event, UdpRuntime},
    tx_ack,
    tx_ack::Error as TxAckErr,
    CodingRate, MacAddress, Modulation,
};
use slog::{debug, info, o, warn, Logger};
use std::{
    convert::TryFrom,
    time::{Duration, Instant},
};

pub const DOWNLINK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct BeaconResp {
    pub powe: i32,
    pub tmst: u32,
}

#[derive(Debug)]
pub enum Message {
    Downlink(Packet),
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
    pub async fn downlink(&self, packet: Packet) {
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
    messages: MessageReceiver,
    uplinks: router::MessageSender,
    beacons: beaconer::MessageSender,
    downlink_mac: MacAddress,
    udp_runtime: UdpRuntime,
    listen_address: String,
    region_watch: region_watcher::MessageReceiver,
    region_params: Option<RegionParams>,
}

impl Gateway {
    pub async fn new(
        settings: &Settings,
        messages: MessageReceiver,
        region_watch: region_watcher::MessageReceiver,
        uplinks: router::MessageSender,
        beacons: beaconer::MessageSender,
    ) -> Result<Self> {
        let gateway = Gateway {
            messages,
            uplinks,
            beacons,
            downlink_mac: Default::default(),
            listen_address: settings.listen.clone(),
            udp_runtime: UdpRuntime::new(&settings.listen).await.map_err(Box::new)?,
            region_watch,
            region_params: None,
        };
        Ok(gateway)
    }

    pub async fn run(&mut self, shutdown: &triggered::Listener, logger: &Logger) -> Result {
        let logger = logger.new(o!("module" => "gateway"));
        info!(logger, "starting"; "listen" => &self.listen_address);
        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                event = self.udp_runtime.recv() =>
                    self.handle_udp_event(&logger, event).await?,
                message = self.messages.recv() => match message {
                    Some(message) => self.handle_message(&logger, message).await,
                    None => {
                        warn!(logger, "ignoring closed message channel");
                        continue;
                    }
                },
                region_change = self.region_watch.changed() => match region_change {
                    Ok(()) => self.region_params = self.region_watch.borrow().clone(),
                    Err(_) => warn!(logger, "region watch disconnected")
                },
            }
        }
    }

    async fn handle_udp_event(&mut self, logger: &Logger, event: Event) -> Result {
        match event {
            Event::UnableToParseUdpFrame(e, buf) => {
                warn!(
                    logger,
                    "ignoring semtech udp parsing error {e}, raw bytes {buf:?}"
                );
            }
            Event::NewClient((mac, addr)) => {
                info!(logger, "new packet forwarder client: {mac}, {addr}");
                self.downlink_mac = mac;
            }
            Event::UpdateClient((mac, addr)) => {
                info!(logger, "mac existed, but IP updated: {mac}, {addr}")
            }
            Event::ClientDisconnected((mac, addr)) => {
                info!(logger, "disconnected packet forwarder: {mac}, {addr}")
            }
            Event::PacketReceived(rxpk, _gateway_mac) => match Packet::try_from(rxpk) {
                Ok(packet) if packet.is_potential_beacon() => {
                    self.beacons.received_beacon(packet).await
                }
                Ok(packet) => self.handle_uplink(logger, packet, Instant::now()).await,
                Err(err) => {
                    warn!(logger, "ignoring push_data: {err:?}");
                }
            },
            Event::NoClientWithMac(_packet, mac) => {
                info!(logger, "ignoring send to client with unknown MAC: {mac}")
            }
            Event::StatReceived(stat, mac) => {
                debug!(logger, "mac: {mac}, stat: {stat:?}")
            }
        };
        Ok(())
    }

    async fn handle_uplink(&mut self, logger: &Logger, packet: Packet, received: Instant) {
        info!(logger, "uplink {} from {}", packet, self.downlink_mac);
        self.uplinks.uplink(packet, received).await;
    }

    async fn handle_message(&mut self, logger: &Logger, message: Message) {
        match message {
            Message::Downlink(packet) => self.handle_downlink(logger, packet).await,
            Message::TransmitBeacon(beacon, tx_resp) => {
                self.handle_transmit_beacon(logger, beacon, tx_resp).await
            }
        }
    }

    fn max_tx_power(&mut self) -> Result<u32> {
        let region_params = self
            .region_params
            .as_ref()
            .ok_or_else(RegionError::no_region_params)?;

        region_params
            .max_tx_power()
            .ok_or_else(RegionError::no_region_tx_power)
    }

    async fn handle_transmit_beacon(
        &mut self,
        logger: &Logger,
        beacon: Beacon,
        responder: sync::ResponseSender<Result<BeaconResp>>,
    ) {
        let tx_power = match self.max_tx_power() {
            Ok(tx_power) => tx_power,
            Err(err) => {
                warn!(logger, "ignoring transmit: {err}");
                responder.send(Err(err), logger);
                return;
            }
        };

        let packet = match beacon_to_pull_resp(&beacon, tx_power as u64) {
            Ok(packet) => packet,
            Err(err) => {
                warn!(logger, "failed to construct beacon pull resp: {err:?}");
                responder.send(Err(err), logger);
                return;
            }
        };

        let beacon_tx = self.udp_runtime.prepare_downlink(packet, self.downlink_mac);

        let logger = logger.clone();
        tokio::spawn(async move {
            let beacon_id = beacon.beacon_id();
            match beacon_tx.dispatch(Some(DOWNLINK_TIMEOUT)).await {
                Ok(tmst) => {
                    info!(logger, "beacon transmitted"; 
                        "beacon" => &beacon_id, 
                        "power" => tx_power, 
                        "tmst" => tmst);
                    responder.send(
                        Ok(BeaconResp {
                            powe: tx_power as i32,
                            tmst: tmst.unwrap_or(0),
                        }),
                        &logger,
                    );
                    tmst
                }
                Err(err) => {
                    if let semtech_udp::server_runtime::Error::Ack(
                        tx_ack::Error::AdjustedTransmitPower(power_used, tmst),
                    ) = err
                    {
                        match power_used {
                            None => {
                                warn!(logger, "packet transmitted with adjusted power, but packet forwarder does not indicate power used.");
                                responder.send(Err(GatewayError::NoBeaconTxPower.into()), &logger);
                            }
                            Some(actual_power) => {
                                info!(logger, "beacon transmitted with adjusted power output"; "beacon" => &beacon_id, "power" => actual_power, "tmst" => tmst);
                                responder.send(
                                    Ok(BeaconResp {
                                        powe: actual_power,
                                        tmst: tmst.unwrap_or(0),
                                    }),
                                    &logger,
                                );
                            }
                        }
                        tmst
                    } else {
                        warn!(logger, "failed to transmit beacon:  {err:?}"; "beacon" => &beacon_id);
                        responder.send(Err(GatewayError::BeaconTxFailure.into()), &logger);
                        None
                    }
                }
            }
        });
    }

    async fn handle_downlink(&mut self, logger: &Logger, downlink: Packet) {
        let tx_power = match self.max_tx_power() {
            Ok(tx_power) => tx_power,
            Err(err) => {
                warn!(logger, "ignoring transmit: {err}");
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
        let logger = logger.clone();

        tokio::spawn(async move {
            if let Ok(txpk) = downlink.to_rx1_pull_resp(tx_power) {
                info!(logger, "rx1 downlink {txpk} via {downlink_mac}",);

                downlink_rx1.set_packet(txpk);
                match downlink_rx1.dispatch(Some(DOWNLINK_TIMEOUT)).await {
                    // On a too early or too late error retry on the rx2 slot if available.
                    Err(SemtechError::Ack(TxAckErr::TooEarly | TxAckErr::TooLate)) => {
                        if let Ok(Some(txpk)) = downlink.to_rx2_pull_resp(tx_power) {
                            info!(logger, "rx2 downlink {txpk} via {downlink_mac}");

                            downlink_rx2.set_packet(txpk);
                            match downlink_rx2.dispatch(Some(DOWNLINK_TIMEOUT)).await {
                                Err(SemtechError::Ack(TxAckErr::AdjustedTransmitPower(_, _))) => {
                                    warn!(logger, "rx2 downlink sent with adjusted transmit power");
                                }
                                Err(err) => warn!(logger, "ignoring rx2 downlink error: {err:?}"),
                                _ => (),
                            }
                        }
                    }
                    Err(SemtechError::Ack(TxAckErr::AdjustedTransmitPower(_, _))) => {
                        warn!(logger, "rx1 downlink sent with adjusted transmit power");
                    }
                    Err(err) => {
                        warn!(logger, "ignoring rx1 downlink error: {:?}", err);
                    }
                    Ok(_) => (),
                }
            }
        });
    }
}

pub fn beacon_to_pull_resp(beacon: &Beacon, tx_power: u64) -> Result<pull_resp::TxPk> {
    // TODO: safe assumption to assume these will always match the used
    // subset?
    let datr = beacon.datarate.to_string().parse().unwrap();
    // convert hz to mhz
    let freq = beacon.frequency as f64 / 1e6;
    let data: Vec<u8> = PHYPayload::proprietary(beacon.data.as_slice()).try_into()?;

    Ok(pull_resp::TxPk {
        imme: true,
        ipol: false,
        modu: Modulation::LORA,
        codr: CodingRate::_4_5,
        datr,
        freq,
        data: pull_resp::PhyData::new(data),
        powe: tx_power,
        rfch: 0,
        tmst: None,
        tmms: None,
        fdev: None,
        prea: None,
        ncrc: None,
    })
}
