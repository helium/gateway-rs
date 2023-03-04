use crate::{
    gateway,
    message_cache::{CacheMessage, MessageCache},
    region_watcher,
    service::packet_router::PacketRouterService,
    sync, Base64, Keypair, MsgSign, Packet, RegionParams, Result, Settings,
};
use exponential_backoff::Backoff;
use helium_proto::services::router::{PacketRouterPacketDownV1, PacketRouterPacketUpV1};
use std::{sync::Arc, time::Instant as StdInstant};
use tokio::time::{self, Duration, Instant};
use tracing::{debug, info, warn};

const STORE_GC_INTERVAL: Duration = Duration::from_secs(60);

const RECONNECT_BACKOFF_RETRIES: u32 = 20;
const RECONNECT_BACKOFF_MIN_WAIT: Duration = Duration::from_secs(5);
const RECONNECT_BACKOFF_MAX_WAIT: Duration = Duration::from_secs(1800); // 30 minutes

#[derive(Debug)]
pub enum Message {
    Uplink {
        packet: Packet,
        received: StdInstant,
    },
}

pub type MessageSender = sync::MessageSender<Message>;
pub type MessageReceiver = sync::MessageReceiver<Message>;

pub fn message_channel() -> (MessageSender, MessageReceiver) {
    sync::message_channel(20)
}

impl MessageSender {
    pub async fn uplink(&self, packet: Packet, received: StdInstant) {
        self.send(Message::Uplink { packet, received }).await
    }
}

pub struct PacketRouter {
    messages: MessageReceiver,
    region_watch: region_watcher::MessageReceiver,
    transmit: gateway::MessageSender,
    service: PacketRouterService,
    reconnect_retry: u32,
    region_params: RegionParams,
    keypair: Arc<Keypair>,
    store: MessageCache<Packet>,
}

impl PacketRouter {
    pub fn new(
        settings: &Settings,
        messages: MessageReceiver,
        region_watch: region_watcher::MessageReceiver,
        transmit: gateway::MessageSender,
    ) -> Self {
        let router_settings = &settings.router;
        let service =
            PacketRouterService::new(router_settings.uri.clone(), settings.keypair.clone());
        let store = MessageCache::new(router_settings.queue);
        let region_params = region_watcher::current_value(&region_watch);
        Self {
            service,
            region_params,
            region_watch,
            keypair: settings.keypair.clone(),
            transmit,
            messages,
            store,
            reconnect_retry: 0,
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn run(&mut self, shutdown: &triggered::Listener) -> Result {
        info!(uri = %self.service.uri, "starting");

        let reconnect_backoff = Backoff::new(
            RECONNECT_BACKOFF_RETRIES,
            RECONNECT_BACKOFF_MIN_WAIT,
            RECONNECT_BACKOFF_MAX_WAIT,
        );

        // Use a deadline based sleep for reconnect to allow the store gc timer
        // to fire without resetting the reconnect timer
        let mut reconnect_sleep = Instant::now() + RECONNECT_BACKOFF_MIN_WAIT;

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!("shutting down");
                    return Ok(())
                },
                message = self.messages.recv() => match message {
                    Some(Message::Uplink{packet, received}) =>
                        self.handle_uplink(packet, received).await,
                    None => warn!("ignoring closed message channel"),
                },
                region_change = self.region_watch.changed() => match region_change {
                    Ok(()) => self.region_params = region_watcher::current_value(&self.region_watch),
                    Err(_) => warn!("region watch disconnected")
                },
                _ = time::sleep_until(reconnect_sleep) => {
                    reconnect_sleep = self.handle_reconnect(&reconnect_backoff).await;
                },
                downlink = self.service.recv() => match downlink {
                    Ok(Some(message)) => self.handle_downlink(message).await,
                    Ok(None) => warn!("router disconnected"),
                    Err(err) => warn!("router error {:?}", err),
                }
            }
        }
    }

    async fn handle_reconnect(&mut self, reconnect_backoff: &Backoff) -> Instant {
        info!("reconnecting");
        match self.service.reconnect().await {
            Ok(_) => {
                info!("reconnected");
                self.reconnect_retry = RECONNECT_BACKOFF_RETRIES;
                self.send_waiting_packets().await
            }
            Err(err) => {
                warn!(%err, "failed to reconnect");
                if self.reconnect_retry == RECONNECT_BACKOFF_RETRIES {
                    self.reconnect_retry = 0;
                } else {
                    self.reconnect_retry += 1;
                }
            }
        }
        Instant::now()
            + reconnect_backoff
                .next(self.reconnect_retry)
                .unwrap_or(RECONNECT_BACKOFF_MAX_WAIT)
    }

    async fn handle_uplink(&mut self, uplink: Packet, received: StdInstant) {
        self.store.push_back(uplink, received);
        self.send_waiting_packets().await;
    }

    async fn handle_downlink(&mut self, message: PacketRouterPacketDownV1) {
        match Packet::try_from(message) {
            Ok(packet) => self.transmit.downlink(packet).await,
            Err(err) => warn!(%err, "could not convert packet to downlink"),
        };
    }

    async fn send_waiting_packets(&mut self) {
        while let (removed, Some(packet)) = self.store.pop_front(STORE_GC_INTERVAL) {
            if removed > 0 {
                info!("discarded {removed} queued packets");
            }
            if let Err(err) = self.send_packet(packet).await {
                warn!(%err, "failed to send uplink")
            }
        }
    }

    pub async fn mk_uplink(&self, packet: CacheMessage<Packet>) -> Result<PacketRouterPacketUpV1> {
        let mut uplink: PacketRouterPacketUpV1 = packet.into_inner().try_into()?;
        uplink.region = self.region_params.region.into();
        uplink.gateway = self.keypair.public_key().into();
        uplink.signature = uplink.sign(self.keypair.clone()).await?;
        Ok(uplink)
    }

    async fn send_packet(&mut self, packet: CacheMessage<Packet>) -> Result {
        debug!(packet_hash = packet.hash().to_b64(), "sending packet");

        let uplink = self.mk_uplink(packet).await?;
        self.service.send(uplink).await
    }
}
