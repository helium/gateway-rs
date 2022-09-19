use crate::{
    beaconing, router::dispatcher, Error, Packet, RawPacket, RegionParams, Result, Settings,
};
use futures::TryFutureExt;
use semtech_udp::{
    server_runtime::{Error as SemtechError, Event, UdpRuntime},
    tx_ack, MacAddress,
};
use slog::{debug, error, info, o, warn, Logger};
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
    TransmitRaw(RawPacket),
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

    /// Send a non-inverted (`ipol = false`) packet that is receivable
    /// by other gateways.
    ///
    /// Essentially, this packet looks like a regular uplink packet to
    /// other gateways until further inspection.
    ///
    /// TODO: can we refactor downlink into a more generic `send` that
    ///       handles packets with `IPOL == true|false`?
    pub async fn transmit_raw(&self, packet: RawPacket) -> Result {
        self.0
            .send(Message::TransmitRaw(packet))
            .map_err(|_| Error::channel())
            .await
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
    beaconing_sender: beaconing::MessageSender,
    downlink_mac: MacAddress,
    udp_runtime: UdpRuntime,
    listen_address: String,
    region_params: Option<RegionParams>,
}

impl Gateway {
    pub async fn new(
        uplinks: dispatcher::MessageSender,
        messages: MessageReceiver,
        beaconing_sender: beaconing::MessageSender,
        settings: &Settings,
    ) -> Result<Self> {
        let gateway = Gateway {
            uplinks,
            downlink_mac: Default::default(),
            messages,
            beaconing_sender,
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
                Ok(packet) if packet.is_longfi() => {
                    info!(logger, "ignoring longfi packet");
                }
                Ok(packet) => {
                    if self
                        .beaconing_sender
                        .send(beaconing::Message::RxPk(packet.clone()))
                        .await
                        .is_err()
                    {
                        error!(logger, "beaconer channel closed")
                    };
                    self.handle_uplink(logger, packet, Instant::now()).await;
                }
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
            Message::TransmitRaw(packet) => self.handle_raw_tx(logger, packet).await,
            Message::RegionParamsChanged(region_params) => {
                if self
                    .beaconing_sender
                    .send(beaconing::Message::RegionParamsChanged(
                        region_params.clone(),
                    ))
                    .await
                    .is_err()
                {
                    error!(logger, "beaconer channel closed")
                };
                self.region_params = Some(region_params);
                info!(logger, "updated region";
                    "region" => RegionParams::to_string(&self.region_params));
            }
        }
    }

    async fn handle_raw_tx(&mut self, logger: &Logger, mut packet: RawPacket) {
        let region_params = if let Some(region_params) = &self.region_params {
            region_params
        } else {
            warn!(logger, "ignoring transmit request, no region params");
            return;
        };

        packet.power_dbm = if let Some(tx_power) = region_params.tx_power() {
            tx_power
        } else {
            warn!(logger, "ignoring transmit request, no tx power");
            return;
        };

        let txpk = packet.into_pull_resp();
        let tx_dl = self
            .udp_runtime
            .prepare_downlink(txpk.clone(), self.downlink_mac);
        let logger = logger.clone();

        tokio::spawn(async move {
            match tx_dl
                .dispatch(Some(Duration::from_secs(DOWNLINK_TIMEOUT_SECS)))
                .await
            {
                Ok(()) => info!(logger, "raw transmit packet {}", txpk),
                Err(e) => warn!(logger, "raw transmit packet, error {}, {}", txpk, e),
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
