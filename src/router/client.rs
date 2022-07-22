use crate::{
    error::Error,
    gateway,
    router::{QuePacket, RouterStore},
    service::router::RouterService,
    state_channel::StateChannelMessage,
    Base64, CacheSettings, KeyedUri, Keypair, Packet, Region, Result,
};
use futures::TryFutureExt;
use slog::{debug, info, o, warn, Logger};
use std::sync::Arc;
use tokio::{
    sync::mpsc,
    time::{self, Duration, MissedTickBehavior},
};

pub const STORE_GC_INTERVAL: Duration = Duration::from_secs(60);
pub const STATE_CHANNEL_CONNECT_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub enum Message {
    Uplink(Packet),
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

    pub async fn uplink(&self, packet: Packet) -> Result {
        self.0
            .send(Message::Uplink(packet))
            .map_err(|_| Error::channel())
            .await
    }

    pub async fn stop(&self) {
        let _ = self.0.send(Message::Stop).await;
    }
}

pub struct RouterClient {
    router: RouterService,
    oui: u32,
    region: Region,
    keypair: Arc<Keypair>,
    downlinks: gateway::MessageSender,
    store: RouterStore,
}

impl RouterClient {
    pub async fn new(
        oui: u32,
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
            oui,
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
            "oui" => self.oui,
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
                    Some(Message::Uplink(packet)) => {
                        self.handle_uplink(&logger, packet)
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
                }
            }
        }
    }

    async fn handle_uplink(&mut self, logger: &Logger, uplink: Packet) -> Result {
        self.store.store_waiting_packet(uplink)?;
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
            if let Some(message) = self.send_packet(logger, &packet).await? {
                match message.to_downlink() {
                    Ok(Some(packet)) => self.handle_downlink(logger, packet).await,
                    Ok(None) => (),
                    Err(err) => warn!(logger, "ignoring router response: {err:?}"),
                }
            }
        }
        Ok(())
    }

    async fn send_packet(
        &mut self,
        logger: &Logger,
        packet: &QuePacket,
    ) -> Result<Option<StateChannelMessage>> {
        debug!(logger, "sending packet";
            "packet_hash" => packet.hash().to_b64());
        StateChannelMessage::packet(
            packet.packet().clone(),
            self.keypair.clone(),
            &self.region,
            packet.hold_time().as_millis() as u64,
        )
        .and_then(|message| self.router.route(message.to_message()))
        .map_ok(StateChannelMessage::from_message)
        .await
    }
}
