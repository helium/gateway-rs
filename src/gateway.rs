use crate::{beaconer, router::dispatcher, Error, Packet, RegionParams, Result, Settings};
use beacon::Beacon;
use futures::TryFutureExt;
use lorawan::PHYPayload;
use semtech_udp::{
    pull_resp,
    server_runtime::{Error as SemtechError, Event, UdpRuntime},
    tx_ack, CodingRate, MacAddress, Modulation,
};
use slog::{debug, info, o, warn, Logger};
use std::{
    convert::TryFrom,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub const DOWNLINK_TIMEOUT_SECS: u64 = 5;
pub const UPLINK_TIMEOUT_SECS: u64 = 6;

#[derive(Debug)]
pub enum Message {
    Downlink(Packet),
    TransmitBeacon(Beacon),
    RegionParamsChanged(RegionParams),
}

#[derive(Clone, Debug)]
pub struct MessageSender(mpsc::Sender<Message>);
pub type MessageReceiver = mpsc::Receiver<Message>;

pub fn message_channel(size: usize) -> (MessageSender, MessageReceiver) {
    let (tx, rx) = mpsc::channel(size);
    (MessageSender(tx), rx)
}

impl MessageSender {
    pub async fn downlink(&self, packet: Packet) -> Result {
        self.0
            .send(Message::Downlink(packet))
            .map_err(|_| Error::channel())
            .await
    }

    /// Send a non-inverted (`ipol = false`) beacon packet that is receivable by
    /// other gateways.
    ///
    /// Essentially, this packet looks like a regular uplink packet to other
    /// gateways until further inspection.
    pub async fn transmit_beacon(&self, beacon: Beacon) {
        let _ = self
            .0
            .send(Message::TransmitBeacon(beacon))
            .map_err(|_| Error::channel())
            .await;
    }

    pub async fn region_params_changed(&self, region_params: RegionParams) {
        let _ = self
            .0
            .send(Message::RegionParamsChanged(region_params))
            .await;
    }
}

pub struct Gateway {
    uplinks: dispatcher::MessageSender,
    messages: MessageReceiver,
    beacon_handler: beaconer::MessageSender,
    downlink_mac: MacAddress,
    udp_runtime: UdpRuntime,
    listen_address: String,
    region_params: Option<RegionParams>,
}

impl Gateway {
    pub async fn new(
        uplinks: dispatcher::MessageSender,
        messages: MessageReceiver,
        beacon_handler: beaconer::MessageSender,
        settings: &Settings,
    ) -> Result<Self> {
        let gateway = Gateway {
            uplinks,
            downlink_mac: Default::default(),
            messages,
            beacon_handler,
            listen_address: settings.listen.clone(),
            udp_runtime: UdpRuntime::new(&settings.listen).await?,
            region_params: None,
        };
        Ok(gateway)
    }

    pub async fn run(&mut self, shutdown: triggered::Listener, logger: &Logger) -> Result {
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
                        warn!(logger, "ignoring closed downlinks channel");
                        continue;
                    }
                }
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
                    self.beacon_handler.received_beacon(packet).await
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
        match self.uplinks.uplink(packet, received).await {
            Ok(()) => (),
            Err(err) => warn!(logger, "ignoring uplink error {:?}", err),
        }
    }

    async fn handle_message(&mut self, logger: &Logger, message: Message) {
        match message {
            Message::Downlink(packet) => self.handle_downlink(logger, packet).await,
            Message::TransmitBeacon(beacon) => self.handle_transmit_beacon(logger, beacon).await,
            Message::RegionParamsChanged(region_params) => {
                self.beacon_handler
                    .region_params_changed(region_params.clone())
                    .await;
                self.region_params = Some(region_params);
                info!(logger, "updated region";
                    "region" => RegionParams::to_string(&self.region_params));
            }
        }
    }

    async fn handle_transmit_beacon(&mut self, logger: &Logger, beacon: Beacon) {
        let region_params = if let Some(region_params) = &self.region_params {
            region_params
        } else {
            warn!(logger, "ignoring transmit request, no region params");
            return;
        };

        let tx_power = if let Some(tx_power) = region_params.tx_power() {
            tx_power
        } else {
            warn!(logger, "ignoring beacon transmit, no tx power");
            return;
        };

        let packet = match beacon_to_pull_resp(&beacon, tx_power as u64) {
            Ok(packet) => packet,
            Err(err) => {
                warn!(logger, "failed to construct beacon pull resp: {err:?}");
                return;
            }
        };

        let beacon_tx = self.udp_runtime.prepare_downlink(packet, self.downlink_mac);

        let logger = logger.clone();
        tokio::spawn(async move {
            let beacon_id = beacon.beacon_id();
            match beacon_tx
                .dispatch(Some(Duration::from_secs(DOWNLINK_TIMEOUT_SECS)))
                .await
            {
                Ok(()) => info!(logger, "beacon transmitted"; "beacon" => &beacon_id),
                Err(err) => {
                    warn!(logger, "failed to transmit beacon:  {err:?}"; "beacon" => &beacon_id)
                }
            };
        });
    }

    async fn handle_downlink(&mut self, logger: &Logger, downlink: Packet) {
        let region_params = if let Some(region_params) = &self.region_params {
            region_params
        } else {
            warn!(logger, "ignoring downlink, no region params");
            return;
        };
        let tx_power = if let Some(tx_power) = region_params.tx_power() {
            tx_power
        } else {
            warn!(logger, "ignoring downlink, no tx power");
            return;
        };
        let (mut downlink_rx1, mut downlink_rx2) = (
            // first downlink
            self.udp_runtime.prepare_empty_downlink(self.downlink_mac),
            // 2nd downlink window if requested by the router response
            self.udp_runtime.prepare_empty_downlink(self.downlink_mac),
        );
        let logger = logger.clone();
        tokio::spawn(async move {
            match downlink.to_pull_resp(false, tx_power).unwrap() {
                None => (),
                Some(txpk) => {
                    info!(
                        logger,
                        "rx1 downlink {} via {}",
                        txpk,
                        downlink_rx1.get_destination_mac()
                    );
                    downlink_rx1.set_packet(txpk);
                    match downlink_rx1
                        .dispatch(Some(Duration::from_secs(DOWNLINK_TIMEOUT_SECS)))
                        .await
                    {
                        // On a too early or too late error retry on the rx2 slot if available.
                        Err(SemtechError::Ack(tx_ack::Error::TooEarly))
                        | Err(SemtechError::Ack(tx_ack::Error::TooLate)) => {
                            if let Some(txpk) = downlink.to_pull_resp(true, tx_power).unwrap() {
                                info!(
                                    logger,
                                    "rx2 downlink {} via {}",
                                    txpk,
                                    downlink_rx2.get_destination_mac()
                                );
                                downlink_rx2.set_packet(txpk);
                                if let Err(err) = downlink_rx2
                                    .dispatch(Some(Duration::from_secs(DOWNLINK_TIMEOUT_SECS)))
                                    .await
                                {
                                    warn!(logger, "ignoring rx2 downlink error: {:?}", err);
                                }
                            }
                        }
                        Err(err) => {
                            warn!(logger, "ignoring rx1 downlink error: {:?}", err);
                        }
                        Ok(()) => (),
                    }
                }
            }
        });
    }
}

pub fn beacon_to_pull_resp(beacon: &Beacon, tx_power: u64) -> Result<pull_resp::TxPk> {
    let size = beacon.data.len() as u64;
    // TODO: safe assumption to assume these will always match the used
    // subset?
    let datr = beacon.datarate.to_string().parse().unwrap();
    // convert hz to mhz
    let freq = beacon.frequency as f64 / 1e6;
    let data = PHYPayload::proprietary(beacon.data.as_slice()).try_into()?;

    Ok(pull_resp::TxPk {
        imme: true,
        ipol: false,
        modu: Modulation::LORA,
        codr: CodingRate::_4_5,
        datr,
        freq,
        data,
        size,
        powe: tx_power,
        rfch: 0,
        tmst: None,
        tmms: None,
        fdev: None,
        prea: None,
        ncrc: None,
    })
}
