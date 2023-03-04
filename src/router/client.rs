use crate::{
    error::Error,
    gateway,
    message_cache::{CacheMessage, MessageCache},
    region_watcher,
    router::StateChannelMessage,
    service::router::RouterService,
    Base64, KeyedUri, Keypair, Packet, RegionParams, Result,
};
use futures::TryFutureExt;
use std::{sync::Arc, time::Instant};
use tokio::{sync::mpsc, time::Duration};
use tracing::{debug, info, warn};

pub const STORE_GC_INTERVAL: Duration = Duration::from_secs(60);
pub const STATE_CHANNEL_CONNECT_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub enum Message {
    Uplink { packet: Packet, received: Instant },
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
    oui: u32,
    region_params: RegionParams,
    region_watch: region_watcher::MessageReceiver,
    keypair: Arc<Keypair>,
    downlinks: gateway::MessageSender,
    store: MessageCache<Packet>,
}

impl RouterClient {
    pub async fn new(
        oui: u32,
        region_watch: region_watcher::MessageReceiver,
        uri: KeyedUri,
        downlinks: gateway::MessageSender,
        keypair: Arc<Keypair>,
        max_packets: u16,
    ) -> Result<Self> {
        let router = RouterService::new(uri)?;
        let store = MessageCache::new(max_packets);
        let region_params = region_watcher::current_value(&region_watch);
        Ok(Self {
            router,
            oui,
            region_watch,
            region_params,
            keypair,
            downlinks,
            store,
        })
    }

    #[tracing::instrument(skip_all, fields(oui = self.oui))]
    pub async fn run(
        &mut self,
        mut messages: MessageReceiver,
        shutdown: triggered::Listener,
    ) -> Result {
        info!(
            uri = %self.router.uri.uri,
            pubkey = %self.router.uri.pubkey,
             "starting"
        );

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!("shutting down");
                    return Ok(())
                },
                message = messages.recv() => match message {
                    Some(Message::Uplink{packet, received}) => {
                        self.handle_uplink(packet, received)
                            .unwrap_or_else(|err| warn!(%err, "ignoring failed uplink"))
                            .await;
                    },
                    Some(Message::Stop) => {
                        info!("stop requested, shutting down");
                        return Ok(())
                    },
                    None => warn!("ignoring closed uplinks channel"),
                },
                region_change = self.region_watch.changed() => match region_change {
                    Ok(()) => {
                        self.region_params = region_watcher::current_value(&self.region_watch);
                        info!(region = %self.region_params, "region updated");
                    },
                    Err(_) => warn!("region watch disconnected"),
                },
            }
        }
    }

    async fn handle_uplink(&mut self, uplink: Packet, received: Instant) -> Result {
        self.store.push_back(uplink, received);
        self.send_waiting_packets().await
    }

    async fn handle_downlink(&mut self, packet: Packet) {
        self.downlinks.downlink(packet).await;
    }

    async fn send_waiting_packets(&mut self) -> Result {
        while let (removed, Some(packet)) = self.store.pop_front(STORE_GC_INTERVAL) {
            if removed > 0 {
                info!("discarded {removed} queued packets");
            }
            if let Some(message) = self.send_packet(packet).await? {
                match message.to_downlink() {
                    Ok(Some(packet)) => self.handle_downlink(packet).await,
                    Ok(None) => (),
                    Err(err) => warn!(%err, "ignoring router response"),
                }
            }
        }
        Ok(())
    }

    async fn send_packet(
        &mut self,
        packet: CacheMessage<Packet>,
    ) -> Result<Option<StateChannelMessage>> {
        debug!(packet_hash = packet.hash().to_b64(), "sending packet");
        let hold_time = packet.hold_time().as_millis() as u64;
        StateChannelMessage::packet(
            packet.into_inner(),
            self.keypair.clone(),
            self.region_params.region,
            hold_time,
        )
        .and_then(|message| self.router.route(message.to_message()))
        .map_ok(StateChannelMessage::from_message)
        .await
    }
}
