use crate::{
    error::Error,
    gateway,
    router::{QuePacket, RouterStore},
    service::router::RouterService,
    Base64, CacheSettings, KeyedUri, Keypair, MsgSign, Packet, Region, Result,
};
use futures::TryFutureExt;
use helium_proto::services::router::PacketRouterPacketUpV1;
use slog::{debug, info, o, warn, Logger};
use std::{sync::Arc, time::Instant};
use tokio::{
    sync::mpsc,
    time::{self, Duration, MissedTickBehavior},
};

pub const STORE_GC_INTERVAL: Duration = Duration::from_secs(60);
pub const STATE_CHANNEL_CONNECT_INTERVAL: Duration = Duration::from_secs(60);

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
        uri: KeyedUri,
        downlinks: gateway::MessageSender,
        keypair: Arc<Keypair>,
        settings: CacheSettings,
    ) -> Result<Self> {
        let router = RouterService::new(uri)?;
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
            "pubkey" => self.router.uri.pubkey.to_string(),
            "uri" => self.router.uri.uri.to_string(),
        ));
        info!(logger, "starting");

        let mut store_gc_timer = time::interval(STORE_GC_INTERVAL);
        store_gc_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                message = messages.recv() => match message {
                    Some(Message::Uplink{packet, received}) => {
                        self.handle_uplink(&logger, packet, received)
                            .unwrap_or_else(|err| warn!(logger, "ignoring failed uplink {:?}", err))
                            .await;
                    },
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
                downlink_message = self.router.message() => match downlink_message {
                    Ok(Some(message)) => {
                        match Packet::try_from(message) {
                            Ok(packet) => self.handle_downlink(&logger, packet).await,
                            Err(err) => warn!(logger, "could not convert packet to downlink {:?}", err),
                        };
                    },
                    Ok(None) => warn!(logger, "router disconnected"),
                    Err(err) => warn!(logger, "router error {:?}", err),
                }
            }
        }
    }

    async fn handle_uplink(
        &mut self,
        logger: &Logger,
        uplink: Packet,
        received: Instant,
    ) -> Result {
        self.store.store_waiting_packet(uplink, received)?;
        self.send_waiting_packets(logger).await
    }

    async fn handle_downlink(&mut self, logger: &Logger, packet: Packet) {
        let _ = self
            .downlinks
            .downlink(packet)
            .inspect_err(|_| warn!(logger, "failed to push downlink"))
            .await;
    }

    async fn send_waiting_packets(&mut self, logger: &Logger) -> Result {
        while let Some(packet) = self.store.pop_waiting_packet() {
            self.send_packet(logger, &packet).await?
        }
        Ok(())
    }

    async fn send_packet(&mut self, logger: &Logger, packet: &QuePacket) -> Result<()> {
        debug!(logger, "sending packet";
            "packet_hash" => packet.hash().to_b64());
        let mut pr_packet = PacketRouterPacketUpV1 {
            payload: packet.payload.clone(),
            timestamp: packet.timestamp,
            signal_strength: packet.signal_strength,
            frequency: packet.frequency,
            datarate: packet.datarate.clone(),
            snr: packet.snr,
            region: self.region.into(),
            hold_time: packet.hold_time().as_millis() as u64,
            hotspot: self.keypair.public_key().into(),
            signature: vec![],
        };
        pr_packet.signature = pr_packet.sign(self.keypair.clone()).await?;
        self.router.route(pr_packet).await
    }
}
