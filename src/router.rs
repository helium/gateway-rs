use crate::{
    error::RegionError, gateway, region_watcher, service::router::RouterService, sync, Base64,
    Error, Keypair, MsgSign, Packet, Region, RegionParams, Result, Settings,
};
use helium_proto::services::router::{PacketRouterPacketDownV1, PacketRouterPacketUpV1};
use slog::{debug, info, o, warn, Logger};
use std::{collections::VecDeque, sync::Arc, time::Instant};
use tokio::time::{self, Duration};

pub const STORE_GC_INTERVAL: Duration = Duration::from_secs(60);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1800); // 30 minutes

#[derive(Debug)]
pub enum Message {
    Uplink { packet: Packet, received: Instant },
}

pub type MessageSender = sync::MessageSender<Message>;
pub type MessageReceiver = sync::MessageReceiver<Message>;

pub fn message_channel() -> (MessageSender, MessageReceiver) {
    sync::message_channel(20)
}

impl MessageSender {
    pub async fn uplink(&self, packet: Packet, received: Instant) {
        self.send(Message::Uplink { packet, received }).await
    }
}

pub struct Router {
    messages: MessageReceiver,
    region_watch: region_watcher::MessageReceiver,
    transmit: gateway::MessageSender,
    service: RouterService,
    region: Option<Region>,
    keypair: Arc<Keypair>,
    store: RouterStore,
}

pub struct RouterStore {
    waiting_packets: VecDeque<QuePacket>,
    max_packets: u16,
}

#[derive(Debug)]
pub struct QuePacket {
    received: Instant,
    packet: Packet,
}

impl Router {
    pub fn new(
        settings: &Settings,
        messages: MessageReceiver,
        region_watch: region_watcher::MessageReceiver,
        transmit: gateway::MessageSender,
    ) -> Self {
        let router_settings = &settings.router;
        let service = RouterService::new(router_settings.uri.clone(), settings.keypair.clone());
        let store = RouterStore::new(router_settings.queue);
        Self {
            service,
            region: Some(settings.region),
            region_watch,
            keypair: settings.keypair.clone(),
            transmit,
            messages,
            store,
        }
    }

    pub async fn run(&mut self, shutdown: &triggered::Listener, logger: &Logger) -> Result {
        let logger = logger.new(o!(
            "module" => "router",
            "uri" => self.service.uri.to_string(),
        ));
        info!(logger, "starting");

        let mut store_gc_timer = time::interval(STORE_GC_INTERVAL);
        let mut reconnect_timer = time::interval(RECONNECT_INTERVAL);

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                message = self.messages.recv() => match message {
                    Some(Message::Uplink{packet, received}) =>
                        self.handle_uplink(&logger, packet, received).await,
                    None => warn!(logger, "ignoring closed message channel"),
                },
                region_change = self.region_watch.changed() => match region_change {
                    Ok(()) => match *self.region_watch.borrow() {
                        Some(RegionParams { region, ..}) => self.region = Some(region),
                        None => self.region = None,
                    },
                    Err(_) => warn!(logger, "region watch disconnected")
                },
                _ = store_gc_timer.tick() => {
                    let removed = self.store.gc_waiting_packets(STORE_GC_INTERVAL);
                    if removed > 0 {
                        info!(logger, "discarded {} queued packets", removed);
                    }
                },
                _ = reconnect_timer.tick() =>
                    self.handle_reconnect(&logger).await,
                downlink = self.service.recv() => match downlink {
                    Ok(Some(message)) => self.handle_downlink(&logger, message).await,
                    Ok(None) => warn!(logger, "router disconnected"),
                    Err(err) => warn!(logger, "router error {:?}", err),
                }
            }
        }
    }

    async fn handle_reconnect(&mut self, logger: &Logger) {
        info!(logger, "reconnecting");
        match self.service.reconnect().await {
            Ok(_) => {
                info!(logger, "reconnected");
                self.send_waiting_packets(logger).await
            }
            Err(err) => warn!(logger, "could not reconnect {err:?}"),
        }
    }

    async fn handle_uplink(&mut self, logger: &Logger, uplink: Packet, received: Instant) {
        match self.store.store_waiting_packet(uplink, received) {
            Ok(_) => self.send_waiting_packets(logger).await,
            Err(err) => warn!(logger, "ignoring failed uplink {:?}", err),
        }
    }

    async fn handle_downlink(&mut self, logger: &Logger, message: PacketRouterPacketDownV1) {
        match Packet::try_from(message) {
            Ok(packet) => self.transmit.downlink(packet).await,
            Err(err) => warn!(logger, "could not convert packet to downlink {:?}", err),
        };
    }

    async fn send_waiting_packets(&mut self, logger: &Logger) {
        while let Some(packet) = self.store.pop_waiting_packet() {
            match self.send_packet(logger, packet).await {
                Ok(()) => (),
                Err(err) => warn!(logger, "failed to send uplink {err:?}"),
            }
        }
    }

    pub async fn mk_uplink(&self, packet: QuePacket) -> Result<PacketRouterPacketUpV1> {
        let region = self.region.ok_or_else(RegionError::no_region_params)?;

        let mut uplink = PacketRouterPacketUpV1::try_from(packet)?;
        uplink.region = region.into();
        uplink.gateway = self.keypair.public_key().into();
        uplink.signature = uplink.sign(self.keypair.clone()).await?;
        Ok(uplink)
    }

    async fn send_packet(&mut self, logger: &Logger, packet: QuePacket) -> Result {
        debug!(logger, "sending packet";
            "packet_hash" => packet.packet().hash().to_b64());

        let uplink = self.mk_uplink(packet).await?;
        self.service.send(uplink).await
    }
}

impl QuePacket {
    pub fn hold_time(&self) -> Duration {
        self.received.elapsed()
    }

    pub fn packet(&self) -> &Packet {
        &self.packet
    }
}

impl TryFrom<QuePacket> for PacketRouterPacketUpV1 {
    type Error = Error;
    fn try_from(value: QuePacket) -> Result<Self> {
        let hold_time = value.hold_time().as_millis() as u64;
        let mut packet = Self::try_from(value.packet)?;
        packet.hold_time = hold_time;
        Ok(packet)
    }
}

impl RouterStore {
    pub fn new(max_packets: u16) -> Self {
        let waiting_packets = VecDeque::new();
        Self {
            waiting_packets,
            max_packets,
        }
    }

    pub fn store_waiting_packet(&mut self, packet: Packet, received: Instant) -> Result {
        self.waiting_packets
            .push_back(QuePacket { packet, received });
        if self.waiting_packets_len() > self.max_packets as usize {
            self.waiting_packets.pop_front();
        }
        Ok(())
    }

    pub fn pop_waiting_packet(&mut self) -> Option<QuePacket> {
        self.waiting_packets.pop_front()
    }

    pub fn waiting_packets_len(&self) -> usize {
        self.waiting_packets.len()
    }

    /// Removes waiting packets older than the given duration. Returns the number
    /// of packets that were removed.
    pub fn gc_waiting_packets(&mut self, duration: Duration) -> usize {
        let before_len = self.waiting_packets.len();
        self.waiting_packets
            .retain(|packet| packet.received.elapsed() <= duration);
        before_len - self.waiting_packets.len()
    }
}
