use crate::{
    error::{Error, StateChannelError},
    hash_str,
    router::{Dispatch, QuePacket, RouterStore, StateChannelEntry},
    service::gateway::{GatewayService, StateChannelFollowService},
    service::router::{RouterService, StateChannelService},
    state_channel::{
        check_active, StateChannelCausality, StateChannelMessage, StateChannelValidation,
    },
    CacheSettings, KeyedUri, Keypair, MsgSign, Packet, Region, Result, TxnFee, TxnFeeConfig,
};
use futures::{future, TryFutureExt};
use helium_proto::{
    blockchain_state_channel_message_v1::Msg, BlockchainTxnStateChannelCloseV1, CloseState,
    GatewayScFollowStreamedRespV1,
};
use slog::{debug, info, o, warn, Logger};
use std::sync::Arc;
use tokio::{
    sync::mpsc,
    time::{self, Duration, MissedTickBehavior},
};

pub const STORE_GC_INTERVAL: Duration = Duration::from_secs(60);
pub const STATE_CHANNEL_CONNECT_INTERVAL: Duration = Duration::from_secs(60);

pub struct RouterClient {
    router: RouterService,
    oui: u32,
    region: Region,
    keypair: Arc<Keypair>,
    downlinks: mpsc::Sender<Packet>,
    gateway: GatewayService,
    state_channel_follower: StateChannelFollowService,
    store: RouterStore,
    first_uplink: bool,
    state_channel: StateChannelService,
}

impl RouterClient {
    pub async fn new(
        oui: u32,
        region: Region,
        uri: KeyedUri,
        mut gateway: GatewayService,
        downlinks: mpsc::Sender<Packet>,
        keypair: Arc<Keypair>,
        settings: CacheSettings,
    ) -> Result<Self> {
        let mut router = RouterService::new(uri)?;
        let state_channel = router.state_channel()?;
        let state_channel_follower = gateway.follow_sc().await?;
        let store = RouterStore::new(&settings);
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
        })
    }

    pub async fn run(
        &mut self,
        mut uplinks: mpsc::Receiver<Dispatch>,
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
                uplink = uplinks.recv() => match uplink {
                    Some(Dispatch::Packet(packet)) => {
                        self.handle_uplink(&logger, packet)
                            .unwrap_or_else(|err| warn!(logger, "ignoring failed uplink {:?}", err))
                            .await;
                    },
                    Some(Dispatch::GatewayChanged) => {
                        info!(logger, "gateway changed, shutting down");
                        return Ok(())
                    },
                    None => warn!(logger, "ignoring closed uplinks channel"),
                },
                gw_message = self.state_channel_follower.message() => match gw_message {
                    Ok(Some(message)) => {
                        self.handle_state_channel_close_message(&logger, message)
                            .unwrap_or_else(|err| warn!(logger, "ignoring gateway service handling error {:?}", err))
                            .await
                    },
                    // The follower service has closd or errored out. Give up
                    // since the dispatcher will notice the disconnect/error and
                    // reconnect a potentially different gateway
                    Ok(None) => {
                        warn!(logger, "gateway service disconnected, shutting down");
                        return Ok(())
                    },
                    Err(err) => {
                        warn!(logger, "gateway service error, shutting down: {:?}", err);
                        return Ok(())
                    }
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
                        warn!(logger, "state channel disconnected");
                    },
                    Err(err) => {
                        // The state channel connect timer will reconnect the
                        // state channel on the next cycle
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
        if self.first_uplink {
            self.first_uplink = false;
            self.maybe_connect_state_channel(logger).await;
        }
        self.send_packet_offers(logger).await
    }

    async fn handle_state_channel_close_message(
        &mut self,
        logger: &Logger,
        message: GatewayScFollowStreamedRespV1,
    ) -> Result {
        let (txn, remove) =
            if let Some(entry) = self.store.get_state_channel_entry_mut(&message.sc_id) {
                let keypair = &self.keypair;
                match CloseState::from_i32(message.close_state).unwrap() {
                    // File a dispute as soon as we hit the expiration time
                    CloseState::Closable => (
                        entry.in_conflict().then(|| mk_close_txn(keypair, entry)),
                        entry.in_conflict(),
                    ),
                    // This is after the router had it's time to close at the
                    // beginning of the grace period. Close non disputed
                    // state channels
                    CloseState::Closing => (
                        (!entry.in_conflict()).then(|| mk_close_txn(keypair, entry)),
                        !entry.in_conflict(),
                    ),
                    // Done with the state channel, get it out of the cache
                    CloseState::Closed => (None, true),
                    // A state channel was disputed. If we disputed it it would
                    // already have been sent and removed as part of Closing
                    // handling. If it was disputed by someone else we'll file
                    // our close here too to get in on the dispute
                    CloseState::Dispute => (Some(mk_close_txn(keypair, entry)), true),
                }
            } else {
                (None, false)
            };
        if let Some(txn) = txn {
            match self.gateway.close_sc(txn).await {
                Ok(()) => (),
                Err(err) => warn!(logger, "ignoring gateway close_sc error: {:?}", err),
            }
        }
        if remove {
            self.store.remove_state_channel(&message.sc_id);
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
                    self.handle_downlink(logger, &packet).await;
                }
                Ok(())
            }
            Msg::Packet(_) => Err(Error::custom("unexpected state channel packet message")),
            Msg::Offer(_) => Err(Error::custom("unexpected state channel offer message")),
            Msg::Purchase(purchase) => {
                let packet = self.store.dequeue_packet(&purchase.packet_hash);
                if let Some(purchase_sc) = &purchase.sc {
                    let public_key = self.keypair.public_key();
                    match check_active(purchase_sc, &mut self.gateway, &self.store)
                        .await
                        .and_then(|prev_sc| prev_sc.is_valid_upgrade_for(public_key, purchase_sc))
                        .and_then(|(prev_sc, new_sc, causality)| {
                            // Chheck that the purchase is an effect of the
                            // current one to avoid double payment
                            if causality != StateChannelCausality::Cause {
                                Err(StateChannelError::causal_conflict(prev_sc, new_sc))
                            } else if let Some(packet) = packet.as_ref() {
                                let dc_total = purchase_sc.total_dcs();
                                let dc_prev_total = (&prev_sc.sc).total_dcs();
                                let dc_packet = packet.dc_payload();
                                // Check that the dc change between the last
                                // state chanel and the new one is at least
                                // incremented by the dcs for the packet.
                                if (dc_total - dc_prev_total) >= dc_packet {
                                    Ok(new_sc)
                                } else {
                                    Err(StateChannelError::underpaid(new_sc))
                                }
                            } else {
                                // We've discarded the packet previously. Accept
                                // the new purchase.
                                info!(logger, "unexpected purchase, accepting state channel");
                                Ok(new_sc)
                            }
                        }) {
                        Err(Error::StateChannel(err)) => match *err {
                            // Overpaid state channels are ignored
                            StateChannelError::Overpaid { sc, .. } => {
                                warn!(logger, "ignoring overpaid state channel"; 
                                    "sc_id" => sc.id_str());
                                self.store.ignore_state_channel(sc)
                            }
                            // Underpaid state channels are ignored
                            StateChannelError::Underpaid { sc, .. } => {
                                warn!(logger, "ignoring underpaid state channel"; 
                                    "sc_id" => sc.id_str());
                                self.store.ignore_state_channel(sc)
                            }
                            // A previously ignored state channel
                            StateChannelError::Ignored { sc, .. } => {
                                warn!(logger, "ignored purchase state channel"; 
                                    "sc_id" => sc.id_str());
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
                                    "sc_id" => sc.id_str());
                                self.state_channel_follower
                                    .send(sc.id(), sc.owner())
                                    .await?;
                                self.store.store_state_channel(sc)?;
                                let _ = self
                                    .send_packet(logger, packet.as_ref())
                                    .map_err(|err| {
                                        warn!(logger, "ignoring packet send error: {:?}", err)
                                    })
                                    .await;
                                self.send_packet_offers(logger).await
                            }
                            // TODO: Ideally we'd find the state channel that
                            // pays us back to most in the conflict between
                            // prev_sc, new_sc and conflicts_with and keep that
                            // one?
                            StateChannelError::CausalConflict { sc, conflicts_with } => {
                                warn!(logger, "ignoring non-causal purchase";
                                    "sc_id" => sc.id_str());
                                self.store
                                    .store_conflicting_state_channel(sc, conflicts_with)
                            }
                            err => {
                                info!(logger, "ignoring purchase: {:?}", err);
                                Ok(())
                            }
                        },
                        Err(err) => {
                            info!(logger, "ignoring purchase: {:?}", err);
                            Ok(())
                        }
                        Ok(new_sc) => {
                            self.store.store_state_channel(new_sc)?;
                            let _ = self
                                .send_packet(logger, packet.as_ref())
                                .map_err(|err| {
                                    warn!(logger, "ignoring packet send error: {:?}", err)
                                })
                                .await;
                            self.send_packet_offers(logger).await
                        }
                    }
                } else {
                    Ok(())
                }
            }
            Msg::Banner(banner) => {
                // We ignore banners since they're not guaranteed to relate to
                // the first received purchase
                if let Some(banner_sc) = &banner.sc {
                    info!(logger, "received banner (ignored)";
                        "sc_id" => hash_str(&banner_sc.id));
                }
                self.send_packet_offers(logger).await
            }
            Msg::Reject(rejection) => {
                debug!(logger, "packet rejected"; 
                    "packet_hash" => hash_str(&rejection.packet_hash));
                self.store.dequeue_packet(&rejection.packet_hash);
                // We do not receive the hash of the packet that was rejected so
                // we rely on the store cleanup to remove the implied packet.
                // Try to send offers again in case we have space
                self.send_packet_offers(logger).await
            }
        }
    }

    async fn handle_downlink(&mut self, logger: &Logger, packet: &helium_proto::Packet) {
        let _ = self
            .downlinks
            .send(packet.clone().into())
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
            self.send_offer(logger, &packet).await?;
            self.store.queue_packet(packet)?;
            if self.state_channel.capacity() == 0 || self.store.packet_queue_full() {
                return Ok(());
            }
        }
        Ok(())
    }

    async fn send_offer(&mut self, _logger: &Logger, packet: &QuePacket) -> Result {
        match StateChannelMessage::offer(
            packet.packet().clone(),
            &self.keypair,
            self.region.clone(),
        ) {
            Ok(message) => Ok(self.state_channel.send(message.to_message()).await?),
            Err(err) => Err(err),
        }
    }

    async fn send_packet(&mut self, logger: &Logger, packet: Option<&QuePacket>) -> Result {
        if packet.is_none() {
            return Ok(());
        }
        let packet = packet.unwrap();
        debug!(logger, "sending packet"; 
            "packet_hash" => packet.hash_str());
        future::ready(StateChannelMessage::packet(
            packet.packet().clone(),
            &self.keypair,
            self.region.clone(),
            packet.hold_time().as_millis() as u64,
        ))
        .and_then(|message| self.state_channel.send(message.to_message()))
        .await
    }
}

fn mk_close_txn(keypair: &Keypair, entry: &StateChannelEntry) -> BlockchainTxnStateChannelCloseV1 {
    let mut txn = BlockchainTxnStateChannelCloseV1 {
        state_channel: Some(entry.sc.sc.clone()),
        closer: keypair.public_key().into(),
        conflicts_with: None,
        fee: 0,
        signature: vec![],
    };
    if let Some(conflicts_with) = &entry.conflicts_with {
        txn.conflicts_with = Some(conflicts_with.sc.clone());
    }
    let fee_config = TxnFeeConfig::default();
    txn.fee = txn.txn_fee(&fee_config).expect("close txn fee");
    txn.signature = txn.sign(keypair).expect("close txn signature");
    txn
}
