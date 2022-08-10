use crate::{
    error::Error,
    gateway,
    router::{QuePacket, RouterStore},
    service::router::RouterService,
    Base64, CacheSettings, Keypair, Packet, Region, Result,
};
use futures::TryFutureExt;
use helium_proto::services::router::PacketRouterPacketDownV1;
use http::Uri;
use slog::{debug, info, o, warn, Logger};
use std::{sync::Arc, time::Instant};
use tokio::{
    sync::mpsc,
    time::{self, Duration, MissedTickBehavior},
};

pub const STORE_GC_INTERVAL: Duration = Duration::from_secs(60);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1800); // 30 minutes

#[derive(Debug)]
pub enum Message {
    Uplink { packet: Packet, received: Instant },
    RegionChanged(Region),
    Stop,
}

#[derive(Clone, Debug)]
pub struct MessageSender(pub(crate) mpsc::Sender<Message>);
pub type MessageReceiver = mpsc::Receiver<Message>;

pub fn message_channel(size: usize) -> (MessageSender, MessageReceiver) {
    let (tx, rx) = mpsc::channel(size);
    (MessageSender(tx), rx)
}

impl MessageSender {
    pub async fn region_changed(&self, region: Region) {
        let _ = self.0.send(Message::RegionChanged(region)).await;
    }

    pub async fn uplink(&self, packet: Packet, received: Instant) -> Result {
        self.0
            .send(Message::Uplink { packet, received })
            .map_err(|_| Error::channel())
            .await
    }

    pub async fn stop(&self) {
        let _ = self.0.send(Message::Stop).await;
    }
}

pub struct RouterClient {
    router: RouterService,
    region: Region,
    keypair: Arc<Keypair>,
    downlinks: gateway::MessageSender,
    store: RouterStore,
}

impl RouterClient {
    pub async fn new(
        region: Region,
        uri: Uri,
        downlinks: gateway::MessageSender,
        keypair: Arc<Keypair>,
        settings: CacheSettings,
    ) -> Result<Self> {
        let router = RouterService::new(uri, keypair.clone())?;
        let store = RouterStore::new(&settings);
        Ok(Self {
            router,
            region,
            keypair,
            downlinks,
            store,
        })
    }

    pub async fn run(
        &mut self,
        mut messages: MessageReceiver,
        shutdown: triggered::Listener,
        logger: &Logger,
    ) -> Result {
        let logger = logger.new(o!(
            "module" => "router",
            "uri" => self.router.uri.to_string(),
        ));
        info!(logger, "starting");

        let mut store_gc_timer = time::interval(STORE_GC_INTERVAL);
        store_gc_timer.set_missed_tick_behavior(MissedTickBehavior::Burst);

        let mut reconnect_timer = time::interval(RECONNECT_INTERVAL);
        reconnect_timer.set_missed_tick_behavior(MissedTickBehavior::Burst);

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                message = messages.recv() => match message {
                    Some(Message::Uplink{packet, received}) =>
                        self.handle_uplink(&logger, packet, received).await,
                    Some(Message::RegionChanged(region)) => {
                        self.region = region;
                        info!(logger, "updated region";
                            "region" => region);
                    },
                    Some(Message::Stop) => {
                        info!(logger, "stop requested, shutting down");
                        return Ok(())
                    },
                    None => warn!(logger, "ignoring closed uplinks channel"),
                },
                _ = store_gc_timer.tick() => {
                    let removed = self.store.gc_waiting_packets(STORE_GC_INTERVAL);
                    if removed > 0 {
                        info!(logger, "discarded {} queued packets", removed);
                    }
                },
                _ = reconnect_timer.tick() =>
                    self.handle_reconnect(&logger).await,
                downlink_message = self.router.message() => match downlink_message {
                    Ok(Some(message)) => self.handle_downlink(&logger, message).await,
                    Ok(None) => warn!(logger, "router disconnected"),
                    Err(err) => warn!(logger, "router error {:?}", err),
                }
            }
        }
    }

    async fn handle_reconnect(&mut self, logger: &Logger) {
        info!(logger, "reconnecting");
        self.router.disconnect();
        match self.router.connect().await {
            Ok(_) => info!(logger, "reconnected"),
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
            Ok(packet) => self.downlinks.downlink(packet).await,
            Err(err) => warn!(logger, "could not convert packet to downlink {:?}", err),
        };
    }

    async fn send_waiting_packets(&mut self, logger: &Logger) {
        while let Some(packet) = self.store.pop_waiting_packet() {
            match self.send_packet(logger, &packet).await {
                Ok(()) => (),
                Err(err) => warn!(logger, "failed to send uplink {err:?}"),
            }
        }
    }

    async fn send_packet(&mut self, logger: &Logger, packet: &QuePacket) -> Result<()> {
        debug!(logger, "sending packet";
            "packet_hash" => packet.hash().to_b64());

        packet
            .to_uplink(self.keypair.clone(), &self.region)
            .and_then(|up| self.router.route(up))
            .await
    }
}
