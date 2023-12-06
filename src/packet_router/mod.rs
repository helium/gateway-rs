use crate::{
    gateway,
    message_cache::{CacheMessage, MessageCache, MessageHash},
    service::{packet_router::PacketRouterService, AckTimer, Reconnect},
    sync, Base64, PacketUp, PublicKey, Result, Settings,
};
use futures::TryFutureExt;
use helium_proto::services::router::{
    envelope_down_v1, PacketRouterPacketAckV1, PacketRouterPacketDownV1, PacketRouterPacketUpV1,
    PacketRouterSessionOfferV1,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{ops::Deref, time::Instant as StdInstant};
use tokio::time::Duration;
use tracing::{debug, info, warn};

const STORE_GC_INTERVAL: Duration = Duration::from_secs(60);

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
    pub session_key: Option<PublicKey>,
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
    reconnect: Reconnect,
    ack_timer: AckTimer,
    store: MessageCache<PacketUp>,
}

impl MessageHash for PacketUp {
    fn hash(&self) -> Vec<u8> {
        Sha256::digest(&self.payload).to_vec()
    }
}

impl PacketRouter {
    pub fn new(
        settings: &Settings,
        messages: MessageReceiver,
        transmit: gateway::MessageSender,
    ) -> Self {
        let router_settings = &settings.router;
        let service = PacketRouterService::new(
            router_settings.uri.clone(),
            router_settings.ack_timeout(),
            settings.keypair.clone(),
        );
        let store = MessageCache::new(router_settings.queue);
        let reconnect = Reconnect::default();
        let ack_timer = AckTimer::new(router_settings.ack_timeout());
        Self {
            service,
            transmit,
            messages,
            store,
            reconnect,
            ack_timer,
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn run(&mut self, shutdown: &triggered::Listener) -> Result {
        info!(uri = %self.service.uri, "starting");

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!("shutting down");
                    return Ok(())
                },
                message = self.messages.recv() => match message {
                    Some(Message::Uplink{packet, received}) =>
                        if self.handle_uplink(packet, received).await.is_err() {
                            self.service.disconnect();
                            warn!("router disconnected");
                            self.reconnect.update_next_time(true);
                            self.ack_timer.update_next_time(false);
                        },
                    Some(Message::Status(tx_resp)) => {
                        let status = RouterStatus {
                            uri: self.service.uri.clone(),
                            connected: self.service.is_connected(),
                            session_key: self.service.session_key().cloned(),
                        };
                        tx_resp.send(status)
                    }
                    None => warn!("ignoring closed message channel"),
                },
                _ = self.reconnect.wait() => {
                    let reconnect_result = self.handle_reconnect().await;
                    self.reconnect.update_next_time(reconnect_result.is_err());
                    self.ack_timer.update_next_time(reconnect_result.is_ok());
                },
                _ = self.ack_timer.wait() => {
                    warn!("no packet acks received");
                    let reconnect_result = self.handle_reconnect().await;
                    self.reconnect.update_next_time(reconnect_result.is_err());
                    self.ack_timer.update_next_time(reconnect_result.is_ok());
                },
                router_message = self.service.recv() => match router_message {
                    Ok(envelope_down_v1::Data::Packet(message)) => self.handle_downlink(message).await,
                    Ok(envelope_down_v1::Data::SessionOffer(message)) => {
                        let session_result = self.handle_session_offer(message).await;
                        if session_result.is_ok() {
                            // (Re)set retry count to max to maximize time to
                            // next disconnect from service
                            self.reconnect.retry_count = self.reconnect.max_retries;
                        } else {
                            // Failed fto handle session offer, disconnect
                            self.service.disconnect();
                        }
                        self.reconnect.update_next_time(session_result.is_err());
                        self.ack_timer.update_next_time(session_result.is_ok());
                    },
                    Ok(envelope_down_v1::Data::PacketAck(message)) => {
                        self.handle_packet_ack(message).await;
                        self.ack_timer.update_next_time(true);
                    },
                    Err(err) => {
                        warn!(?err, "router error");
                        self.reconnect.update_next_time(true);
                        self.ack_timer.update_next_time(false);
                    },
                }
            }
        }
    }

    async fn handle_reconnect(&mut self) -> Result {
        // Do not send waiting packets on ok here since we wait for a session
        // offer. Also do not reset the reconnect retry counter since only a
        // session key indicates a good connection
        self.service
            .reconnect()
            .inspect_err(|err| warn!(%err, "failed to reconnect"))
            .await
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

    async fn handle_packet_ack(&mut self, message: PacketRouterPacketAckV1) {
        if message.payload_hash.is_empty() {
            // Empty ack is just a heartbeat and is ignored
            return;
        }
        if let Some(index) = self.store.index_of(|msg| msg.hash == message.payload_hash) {
            self.store.remove_to(index);
            debug!(removed = index, "removed acked packets");
        }
    }

    async fn handle_session_offer(&mut self, message: PacketRouterSessionOfferV1) -> Result {
        self.service.session_init(&message.nonce).await?;
        self.send_waiting_packets()
            .inspect_err(|err| warn!(%err, "failed to send queued packets"))
            .await
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

        let mut uplink: PacketRouterPacketUpV1 = packet.deref().into();
        uplink.hold_time = packet.hold_time().as_millis() as u64;
        self.service.send_uplink(uplink).await
    }
}
