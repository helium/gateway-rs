use crate::{
    gateway,
    message_cache::{CacheMessage, MessageCache},
    service::packet_router::PacketRouterService,
    sign, sync, Base64, Keypair, PacketUp, Result, Settings,
};
use exponential_backoff::Backoff;
use helium_proto::{
    services::router::{PacketRouterPacketDownV1, PacketRouterPacketUpV1},
    Message as ProtoMessage,
};
use serde::Serialize;
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
        packet: PacketUp,
        received: StdInstant,
    },
    Status(sync::ResponseSender<RouterStatus>),
}

#[derive(Debug, Clone, Serialize)]
pub struct RouterStatus {
    #[serde(with = "http_serde::uri")]
    pub uri: http::Uri,
    pub connected: bool,
}

pub type MessageSender = sync::MessageSender<Message>;
pub type MessageReceiver = sync::MessageReceiver<Message>;

pub fn message_channel() -> (MessageSender, MessageReceiver) {
    sync::message_channel(20)
}

impl MessageSender {
    pub async fn uplink(&self, packet: PacketUp, received: StdInstant) {
        self.send(Message::Uplink { packet, received }).await
    }

    pub async fn status(&self) -> Result<RouterStatus> {
        self.request(Message::Status).await
    }
}

pub struct PacketRouter {
    messages: MessageReceiver,
    transmit: gateway::MessageSender,
    service: PacketRouterService,
    reconnect_retry: u32,
    keypair: Arc<Keypair>,
    store: MessageCache<PacketUp>,
}

impl PacketRouter {
    pub fn new(
        settings: &Settings,
        messages: MessageReceiver,
        transmit: gateway::MessageSender,
    ) -> Self {
        let router_settings = &settings.router;
        let service =
            PacketRouterService::new(router_settings.uri.clone(), settings.keypair.clone());
        let store = MessageCache::new(router_settings.queue);
        Self {
            service,
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
                        if self.handle_uplink(packet, received).await.is_err() {
                            warn!("router disconnected");
                            reconnect_sleep = self.next_connect(&reconnect_backoff, true);
                        },
                    Some(Message::Status(tx_resp)) => {
                        let status = RouterStatus {
                            uri: self.service.uri.clone(),
                            connected: self.service.is_connected(),
                        };
                        tx_resp.send(status)
                    }
                    None => warn!("ignoring closed message channel"),
                },
                _ = time::sleep_until(reconnect_sleep) => {
                    reconnect_sleep = self.handle_reconnect(&reconnect_backoff).await;
                },
                downlink = self.service.recv() => match downlink {
                    Ok(Some(message)) => self.handle_downlink(message).await,
                    Ok(None) => {
                        warn!("router disconnected");
                        reconnect_sleep = self.next_connect(&reconnect_backoff, true)
                    },
                    Err(err) => {
                        warn!(?err, "router error");
                        reconnect_sleep = self.next_connect(&reconnect_backoff, true)
                    },
                }
            }
        }
    }

    fn next_connect(&mut self, reconnect_backoff: &Backoff, inc_retry: bool) -> Instant {
        if inc_retry {
            if self.reconnect_retry == RECONNECT_BACKOFF_RETRIES {
                self.reconnect_retry = 0;
            } else {
                self.reconnect_retry += 1;
            }
        }
        let backoff = reconnect_backoff
            .next(self.reconnect_retry)
            .unwrap_or(RECONNECT_BACKOFF_MAX_WAIT);
        info!(seconds = backoff.as_secs(), "next connect");
        Instant::now() + backoff
    }

    async fn handle_reconnect(&mut self, reconnect_backoff: &Backoff) -> Instant {
        info!("connecting");
        let inc_retry = match self.service.reconnect().await {
            Ok(_) => {
                info!("connected");
                self.reconnect_retry = RECONNECT_BACKOFF_RETRIES;
                self.send_waiting_packets().await.is_err()
            }
            Err(err) => {
                warn!(%err, "failed to connect");
                true
            }
        };
        self.next_connect(reconnect_backoff, inc_retry)
    }

    async fn handle_uplink(&mut self, uplink: PacketUp, received: StdInstant) -> Result {
        self.store.push_back(uplink, received);
        if self.service.is_connected() {
            self.send_waiting_packets().await?;
        }
        Ok(())
    }

    async fn handle_downlink(&mut self, message: PacketRouterPacketDownV1) {
        self.transmit.downlink(message.into()).await;
    }

    async fn send_waiting_packets(&mut self) -> Result {
        while let (removed, Some(packet)) = self.store.pop_front(STORE_GC_INTERVAL) {
            if removed > 0 {
                info!(removed, "discarded queued packets");
            }
            if let Err(err) = self.send_packet(&packet).await {
                warn!(%err, "failed to send uplink");
                self.store.push_front(packet);
                return Err(err);
            }
        }
        Ok(())
    }

    async fn send_packet(&mut self, packet: &CacheMessage<PacketUp>) -> Result {
        debug!(packet_hash = packet.hash().to_b64(), "sending packet");

        let uplink = mk_uplink(packet, self.keypair.clone()).await?;
        self.service.send(uplink).await
    }
}

pub async fn mk_uplink(
    packet: &CacheMessage<PacketUp>,
    keypair: Arc<Keypair>,
) -> Result<PacketRouterPacketUpV1> {
    use std::ops::Deref;
    let mut uplink: PacketRouterPacketUpV1 = packet.deref().into();
    uplink.hold_time = packet.hold_time().as_millis() as u64;
    uplink.gateway = keypair.public_key().into();
    uplink.signature = sign(keypair, uplink.encode_to_vec()).await?;
    Ok(uplink)
}
