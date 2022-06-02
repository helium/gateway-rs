use crate::{
    error::{Error, StateChannelError},
    gateway,
    router::{QuePacket, RouterStore, StateChannelEntry},
    service::router::{RouterService, StateChannelService},
    service::{
        self,
        gateway::{GatewayService, StateChannelFollowService},
    },
    state_channel::{check_active, check_active_diff, StateChannel, StateChannelMessage},
    Base64, CacheSettings, KeyedUri, Keypair, MsgSign, Packet, Region, Result, TxnFee,
    TxnFeeConfig,
};
use futures::{future::OptionFuture, TryFutureExt};
use helium_proto::{
    blockchain_state_channel_message_v1::Msg, BlockchainStateChannelDiffV1,
    BlockchainStateChannelV1, BlockchainTxnStateChannelCloseV1, CloseState,
};
use slog::{debug, info, o, warn, Logger};
use std::sync::Arc;
use tokio::{
    sync::mpsc,
    time::{self, Duration, MissedTickBehavior},
};
use tokio_stream::StreamExt;

pub const STORE_GC_INTERVAL: Duration = Duration::from_secs(60);
pub const STATE_CHANNEL_CONNECT_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub enum Message {
    Uplink(Packet),
    Region(Region),
    GatewayChanged(Option<GatewayService>),
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
    pub async fn gateway_changed(&self, gateway: Option<GatewayService>) {
        let _ = self.0.send(Message::GatewayChanged(gateway)).await;
    }

    pub async fn region_changed(&self, region: Region) {
        let _ = self.0.send(Message::Region(region)).await;
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
    gateway: Option<GatewayService>,
    state_channel_follower: StateChannelFollowService,
    store: RouterStore,
    // This allows an attempt to connect on an initial uplink without endlessly
    // trying to connect to a failing state channel
    first_uplink: bool,
    // This is used to request state channel diffs on anything but the first
    // offer sent to the state channel
    first_offer: bool,
    state_channel: StateChannelService,
}

impl RouterClient {
    pub async fn new(
        oui: u32,
        region: Region,
        uri: KeyedUri,
        mut gateway: GatewayService,
        downlinks: gateway::MessageSender,
        keypair: Arc<Keypair>,
        settings: CacheSettings,
    ) -> Result<Self> {
        let mut router = RouterService::new(uri)?;
        let state_channel = router.state_channel()?;
        let state_channel_follower = gateway.follow_sc().await?;
        let store = RouterStore::new(&settings);
        let gateway = Some(gateway);
        Ok(Self {
            router,
            oui,
            region,
            keypair,
            downlinks,
            store,
            state_channel,
            gateway,
            state_channel_follower,
            first_uplink: true,
            first_offer: true,
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

        let mut state_channel_connect_timer = time::interval(STATE_CHANNEL_CONNECT_INTERVAL);
        state_channel_connect_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

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
                    Some(Message::GatewayChanged(gateway)) => {
                        info!(logger, "gateway changed");
                        self.gateway = gateway;
                        match self.state_channel_follower.set_gateway(self.gateway.as_mut()).await {
                            Ok(()) => (),
                            Err(err) => {
                                warn!(logger, "ignoring gateway service setup error: {err:?}");
                                let _ = self.state_channel_follower.set_gateway(None).await;
                            }
                        }
                    },
                    Some(Message::Region(region)) => {
                        self.region = region;
                        info!(logger, "updated region to {region}" );
                    },
                    Some(Message::Stop) => {
                        info!(logger, "stop requested, shutting down");
                        return Ok(())
                    },
                    None => warn!(logger, "ignoring closed uplinks channel"),
                },
                gw_message = self.state_channel_follower.next() => match gw_message {
                    Some(Ok(message)) => {
                        self.handle_state_channel_close_message(&logger, &message)
                            .unwrap_or_else(|err| warn!(logger, "ignoring gateway service handling error {:?}", err))
                            .await
                    },
                    Some(Err(err)) => {
                        warn!(logger, "ignoring gateway service error: {:?}", err);
                    }
                    // The follower service has closd or errored out. Give up
                    // since the dispatcher will notice the disconnect/error and
                    // reconnect a potentially different gateway
                    None => {
                        warn!(logger, "gateway service disconnected");
                        let _ = self.state_channel_follower.set_gateway(None).await;
                    },
                },
                sc_message = self.state_channel.message() =>  match sc_message {
                    Ok(Some(message)) => {
                        if let Some(inner_msg) = message.msg {
                            self.handle_state_channel_message(&logger, inner_msg.into())
                                .unwrap_or_else(|err| warn!(logger, "ignoring state channel handling error {:?}", err))
                                .await
                        }
                    },
                    Ok(None) => {
                        // The state channel connect timer will reconnect the
                        // state channel on the next cycle
                        self.first_offer = true;
                        warn!(logger, "state channel disconnected");
                    },
                    Err(err) => {
                        // The state channel connect timer will reconnect the
                        // state channel on the next cycle
                        self.first_offer = true;
                        warn!(logger, "state channel error {:?}", err);
                    },
                },
                _ = store_gc_timer.tick() => {
                    let removed = self.store.gc_queued_packets(STORE_GC_INTERVAL);
                    if removed > 0 {
                        info!(logger, "discarded {} queued packets", removed);
                    }
                }
                _ = state_channel_connect_timer.tick() => {
                    self.maybe_connect_state_channel(&logger).await
                }
            }
        }
    }

    // Reconects the state channel if there are queued or waiting packets in the
    // store for the target router
    async fn maybe_connect_state_channel(&mut self, logger: &Logger) {
        if self.store.packet_queue_len() + self.store.waiting_packets_len() > 0
            && !self.state_channel.is_connected()
        {
            match self.state_channel.connect().await {
                Ok(()) => info!(logger, "connected state channel"),
                Err(err) => warn!(logger, "failed to connect state channel: {:?}", err),
            }
        }
    }

    async fn handle_uplink(&mut self, logger: &Logger, uplink: Packet) -> Result {
        self.store.store_waiting_packet(uplink)?;
        // First uplink is used to get a quicker state channel connect than
        // waiting for the state channel connect timer to trigger
        if self.first_uplink {
            self.first_uplink = false;
            self.maybe_connect_state_channel(logger).await;
        }
        self.send_packet_offers(logger).await
    }

    async fn handle_state_channel_close_message<R: service::gateway::Response>(
        &mut self,
        logger: &Logger,
        message: &R,
    ) -> Result {
        let message = message.state_channel_response()?;
        let (txn, remove): (OptionFuture<_>, bool) = if let Some(entry) =
            self.store.get_state_channel_entry_mut(&message.sc_id)
        {
            let keypair = self.keypair.clone();
            match message.close_state() {
                // File a dispute as soon as we hit the expiration time
                CloseState::Closable => (
                    (entry.in_conflict())
                        .then(|| mk_close_txn(keypair, entry.clone()))
                        .into(),
                    entry.in_conflict(),
                ),
                // This is after the router had it's time to close at the
                // beginning of the grace period. Close non disputed
                // state channels
                CloseState::Closing => (
                    (!entry.in_conflict())
                        .then(|| mk_close_txn(keypair, entry.clone()))
                        .into(),
                    !entry.in_conflict(),
                ),
                // Done with the state channel, get it out of the cache
                CloseState::Closed => (None.into(), true),
                // A state channel was disputed. If we disputed it it would
                // already have been sent and removed as part of Closing
                // handling. If it was disputed by someone else we'll file
                // our close here too to get in on the dispute
                CloseState::Dispute => (Some(mk_close_txn(keypair, entry.clone())).into(), true),
            }
        } else {
            (None.into(), false)
        };
        if remove {
            self.store.remove_state_channel(&message.sc_id);
        }
        if let Some(txn) = txn.await {
            if let Some(gateway) = &mut self.gateway {
                let _ = gateway
                    .close_sc(txn)
                    .inspect_err(|err| warn!(logger, "ignoring gateway close_sc error: {:?}", err))
                    .await;
            } else {
                return Err(Error::no_service());
            }
        }
        Ok(())
    }

    async fn handle_state_channel_message(
        &mut self,
        logger: &Logger,
        message: StateChannelMessage,
    ) -> Result {
        match message.msg() {
            Msg::Response(response) => {
                if let Some(packet) = Packet::from_state_channel_response(response.to_owned()) {
                    self.handle_downlink(logger, packet).await;
                }
                Ok(())
            }
            Msg::Packet(_) => Err(Error::custom("unexpected state channel packet message")),
            Msg::Offer(_) => Err(Error::custom("unexpected state channel offer message")),
            Msg::Purchase(purchase) => {
                let packet = self.store.dequeue_packet(&purchase.packet_hash);
                let packet_ref = packet.as_ref();
                let state_channel_result = if let Some(purchase_sc) = &purchase.sc {
                    self.handle_purchase_state_channel(logger, packet_ref, purchase_sc)
                        .await
                } else if let Some(purchase_sc_diff) = &purchase.sc_diff {
                    self.handle_purchase_state_channel_diff(logger, packet_ref, purchase_sc_diff)
                        .await
                } else {
                    Ok(None)
                };

                match state_channel_result {
                    Err(Error::StateChannel(err)) => match *err {
                        // Overpaid state channels are ignored
                        StateChannelError::Overpaid { sc, .. } => {
                            warn!(logger, "ignoring overpaid state channel"; 
                                    "sc_id" => sc.id().to_b64url());
                            self.store.ignore_state_channel(sc)
                        }
                        // Underpaid state channels are ignored
                        StateChannelError::Underpaid { sc, .. } => {
                            warn!(logger, "ignoring underpaid state channel"; 
                                    "sc_id" => sc.id().to_b64url());
                            self.store.ignore_state_channel(sc)
                        }
                        // A previously ignored state channel
                        StateChannelError::Ignored { sc, .. } => {
                            warn!(logger, "ignored purchase state channel"; 
                                    "sc_id" => sc.id().to_b64url());
                            Ok(())
                        }
                        // A new channel was detected. We have no baseline
                        // for the received state channel in the purchase.
                        // Accept it, follow it for close actions and submit
                        // the packet
                        //
                        // TODO: Ideally we would check if the difference
                        // between last purchase channel (this is the harder
                        // part to infer) and the new one is enough to cover
                        // for the packet.
                        StateChannelError::NewChannel { sc } => {
                            info!(logger, "accepting new state channel";
                                    "sc_id" => sc.id().to_b64url());
                            self.state_channel_follower
                                .send(sc.id(), sc.owner())
                                .await?;
                            self.store.store_state_channel(sc)?;
                            let _ = self
                                .send_packet(logger, packet_ref)
                                .map_err(|err| warn!(logger, "ignoring packet send error: {err:?}"))
                                .await;
                            self.send_packet_offers(logger).await
                        }
                        // TODO: Ideally we'd find the state channel that
                        // pays us back to most in the conflict between
                        // prev_sc, new_sc and conflicts_with and keep that
                        // one?
                        StateChannelError::CausalConflict { sc, conflicts_with } => {
                            warn!(logger, "ignoring non-causal purchase";
                                    "sc_id" => sc.id().to_b64url());
                            self.store
                                .store_conflicting_state_channel(sc, conflicts_with)
                        }
                        StateChannelError::NotFound { sc_id } => {
                            warn!(logger, "accepting purchase with no local state channel";
                                    "sc_id" => sc_id.to_b64url());
                            // Apparently we got an sc_diff on a purchase, but
                            // we have no local knowledge of that state channel.
                            // We tentatively accept the purchase by sending the
                            // packet and request the full state channel in the
                            // next offer.
                            self.first_offer = true;
                            let _ = self
                                .send_packet(logger, packet_ref)
                                .map_err(|err| warn!(logger, "ignoring packet send error: {err:?}"))
                                .await;
                            self.send_packet_offers(logger).await
                        }
                        err => {
                            info!(logger, "ignoring purchase: {err:?}");
                            Ok(())
                        }
                    },
                    Err(err) => {
                        info!(logger, "ignoring purchase: {err:?}");
                        Ok(())
                    }
                    Ok(Some(new_sc)) => {
                        self.store.store_state_channel(new_sc)?;
                        let _ = self
                            .send_packet(logger, packet_ref)
                            .map_err(|err| warn!(logger, "ignoring packet send error: {err:?}"))
                            .await;
                        self.send_packet_offers(logger).await
                    }
                    Ok(None) => Ok(()),
                }
            }
            Msg::Banner(banner) => {
                // We ignore banners since they're not guaranteed to relate to
                // the first received purchase
                if let Some(banner_sc) = &banner.sc {
                    info!(logger, "received banner (ignored)";
                        "sc_id" => banner_sc.id.to_b64url());
                }
                self.send_packet_offers(logger).await
            }
            Msg::Reject(rejection) => {
                debug!(logger, "packet rejected"; 
                    "packet_hash" => rejection.packet_hash.to_b64());
                self.store.dequeue_packet(&rejection.packet_hash);
                // We do not receive the hash of the packet that was rejected so
                // we rely on the store cleanup to remove the implied packet.
                // Try to send offers again in case we have space
                self.send_packet_offers(logger).await
            }
        }
    }

    async fn handle_purchase_state_channel(
        &mut self,
        _logger: &Logger,
        packet: Option<&QuePacket>,
        sc: &BlockchainStateChannelV1,
    ) -> Result<Option<StateChannel>> {
        if let Some(gateway) = &mut self.gateway {
            let public_key = self.keypair.public_key();
            check_active(sc, gateway, &self.store)
                .await
                .and_then(|prev_sc| prev_sc.is_valid_purchase_sc(public_key, packet, sc))
                .map(Some)
        } else {
            Err(Error::no_service())
        }
    }

    async fn handle_purchase_state_channel_diff(
        &mut self,
        _logger: &Logger,
        packet: Option<&QuePacket>,
        sc_diff: &BlockchainStateChannelDiffV1,
    ) -> Result<Option<StateChannel>> {
        let public_key = self.keypair.public_key();
        check_active_diff(sc_diff, &self.store)
            .await
            .and_then(|prev_sc| prev_sc.is_valid_purchase_sc_diff(public_key, packet, sc_diff))
            .map(Some)
    }

    async fn handle_downlink(&mut self, logger: &Logger, packet: Packet) {
        let _ = self
            .downlinks
            .downlink(packet)
            .inspect_err(|_| warn!(logger, "failed to push downlink"))
            .await;
    }

    async fn send_packet_offers(&mut self, logger: &Logger) -> Result {
        if !self.state_channel.is_connected() {
            return Ok(());
        }
        if self.state_channel.capacity() == 0 || self.store.packet_queue_full() {
            return Ok(());
        }
        while let Some(packet) = self.store.pop_waiting_packet() {
            self.send_offer(logger, &packet, self.first_offer).await?;
            self.first_offer = false;
            self.store.queue_packet(packet)?;
            if self.state_channel.capacity() == 0 || self.store.packet_queue_full() {
                return Ok(());
            }
        }
        Ok(())
    }

    async fn send_offer(
        &mut self,
        _logger: &Logger,
        packet: &QuePacket,
        first_offer: bool,
    ) -> Result {
        StateChannelMessage::offer(
            packet.packet().clone(),
            self.keypair.clone(),
            self.region,
            !first_offer,
        )
        .and_then(|message| self.state_channel.send(message.to_message()))
        .await
    }

    async fn send_packet(&mut self, logger: &Logger, packet: Option<&QuePacket>) -> Result {
        if packet.is_none() {
            return Ok(());
        }
        let packet = packet.unwrap();
        debug!(logger, "sending packet";
            "packet_hash" => packet.hash().to_b64());
        StateChannelMessage::packet(
            packet.packet().clone(),
            self.keypair.clone(),
            self.region,
            packet.hold_time().as_millis() as u64,
        )
        .and_then(|message| self.state_channel.send(message.to_message()))
        .await
    }
}

async fn mk_close_txn(
    keypair: Arc<Keypair>,
    entry: StateChannelEntry,
) -> BlockchainTxnStateChannelCloseV1 {
    let mut txn = BlockchainTxnStateChannelCloseV1 {
        state_channel: Some(entry.sc.sc),
        closer: keypair.public_key().into(),
        conflicts_with: None,
        fee: 0,
        signature: vec![],
    };
    if let Some(conflicts_with) = entry.conflicts_with {
        txn.conflicts_with = Some(conflicts_with.sc);
    }
    let fee_config = TxnFeeConfig::default();
    txn.fee = txn.txn_fee(&fee_config).expect("close txn fee");
    txn.signature = txn.sign(keypair).await.expect("signature");
    txn
}
