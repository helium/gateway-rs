use crate::{
    error::{Error, StateChannelError},
    router::{Dispatch, QuePacket, RouterStore, StateChannelEntry},
    service::gateway::{GatewayService, StateChannelFollowService},
    service::router::{RouterService, StateChannelService},
    state_channel::{StateChannelMessage, StateChannelValidation},
    CacheSettings, KeyedUri, Keypair, MsgSign, Packet, Region, Result, TxnFee, TxnFeeConfig,
};
use helium_proto::{
    blockchain_state_channel_message_v1::Msg, BlockchainTxnStateChannelCloseV1, CloseState,
    GatewayScFollowStreamedRespV1,
};
use slog::{debug, info, o, warn, Logger};
use std::{cmp::max, sync::Arc};
use tokio::sync::mpsc;

pub struct RouterClient {
    router: RouterService,
    oui: u32,
    region: Region,
    keypair: Arc<Keypair>,
    downlinks: mpsc::Sender<Packet>,
    gateway: GatewayService,
    state_channel_follower: StateChannelFollowService,
    store: RouterStore,
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
            "public_key" => self.router.uri.public_key.to_string(),
            "uri" => self.router.uri.uri.to_string(),
            "oui" => self.oui,
        ));
        info!(logger, "starting");

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                uplink = uplinks.recv() => match uplink {
                    Some(Dispatch::Packet(packet)) => match self.handle_uplink(&logger, packet).await {
                        Ok(()) =>  (),
                        Err(err) => warn!(logger, "ignoring failed uplink {:?}", err)
                    },
                    Some(Dispatch::Gateway(gateway)) => {
                        info!(logger, "updating gateway";
                            "public_key" => gateway.uri.public_key.to_string(),
                            "uri" => gateway.uri.uri.to_string());
                        self.gateway = gateway;
                    },
                    None => warn!(logger, "ignoring closed uplinks channel"),
                },
                gw_message = self.state_channel_follower.message() => match gw_message {
                    Ok(Some(message)) =>  {
                        match self.handle_state_channel_close_message(&logger, message).await {
                            Ok(()) => (),
                            Err(err) => warn!(logger, "ignoring gateway handling error {:?}", err),
                        }
                    },
                    Ok(None) => return Ok(()),
                    Err(err) => {
                        warn!(logger, "gateway service error {:?}", err);
                        return Ok(())
                    }
                },
                sc_message = self.state_channel.message() =>  match sc_message {
                    Ok(Some(message)) => {
                        if let Some(inner_msg) = message.msg {
                        match self.handle_state_channel_message(&logger, inner_msg.into()).await {
                            Ok(()) => (),
                            Err(err) => warn!(logger, "ignoring state channel handling error {:?}", err),
                        }
                    }
                    },
                    Ok(None) => return Ok(()),
                    Err(err) => {
                        warn!(logger, "state channel error {:?}", err);
                        return Ok(())
                    }
                }
            }
        }
    }

    async fn handle_uplink(&mut self, logger: &Logger, uplink: Packet) -> Result {
        // if self.store.state_channel_count() == 0 {
        //     self.state_channel.connect().await?;
        // }
        // self.send_packet(logger, Some(&QuePacket::from(uplink)))
        //     .await
        self.store.store_waiting_packet(uplink)?;
        if self.store.state_channel_count() == 0 {
            // No banner received yet, start connect
            return self.state_channel.connect().await;
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
                let packet = self.store.deque_packet();
                if let Some(purchase_sc) = &purchase.sc {
                    let public_key = self.keypair.public_key();
                    match purchase_sc
                        .check_active(&mut self.gateway, &self.store)
                        .await
                        .and_then(|sc| sc.is_valid_upgrade_for(public_key, purchase_sc))
                        .and_then(|(prev_sc, new_sc)| {
                            if let Some(packet) = packet.as_ref() {
                                let dc_budget = purchase_sc.credits;
                                let dc_total = purchase_sc.total_dcs();
                                let dc_remaining = max(0, dc_budget - dc_total);

                                // Check that the dcs remaining in the state
                                // channel at least covers the dcs required for
                                // this packet
                                let dc_packet = packet.dc_payload();
                                if dc_remaining >= dc_packet {
                                    let dc_prev_total = (&prev_sc.sc).total_dcs();
                                    // Check that the dc change between the last
                                    // state chanel and the new one is at least
                                    // incremented by the dcs for the packet.
                                    if (dc_total - dc_prev_total) >= dc_packet {
                                        Ok(new_sc)
                                    } else {
                                        Err(StateChannelError::underpaid(new_sc))
                                    }
                                } else {
                                    Err(StateChannelError::low_balance())
                                }
                            } else {
                                // We've discarded the packet previously. Accept
                                // the new purchase.
                                Ok(new_sc)
                            }
                        }) {
                        Err(Error::StateChannel(err)) => match *err {
                            StateChannelError::Overpaid { sc, .. } => {
                                // we don't need to keep a sibling here as proof of misbehaviour is standalone
                                // this will conflict or dominate any later attempt to close within spec
                                self.store.store_state_channel(sc)
                            }
                            StateChannelError::Underpaid { sc, .. } => {
                                // We're not getting paid for this packet. Drop it
                                // and store the channel using the same reasoning as
                                // Overpaid
                                self.store.store_state_channel(sc)
                            }
                            StateChannelError::CausalConflict { sc, conflicts_with } => self
                                .store
                                .store_conflicting_state_channel(sc, conflicts_with),
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
                            let _ =
                                self.send_packet(logger, packet.as_ref())
                                    .await
                                    .map_err(|err| {
                                        warn!(logger, "ignoring packet send error: {:?}", err)
                                    });
                            self.send_packet_offers(logger).await
                        }
                    }
                } else {
                    Ok(())
                }
            }
            Msg::Banner(banner) => {
                if let Some(banner_sc) = &banner.sc {
                    let public_key = self.keypair.public_key();
                    match banner_sc
                        .check_active(&mut self.gateway, &self.store)
                        .await
                        .and_then(|sc| sc.is_valid_upgrade_for(public_key, banner_sc))
                    {
                        Ok((_, new_sc)) => {
                            info!(logger, "received banner";
                                "sc_id" => new_sc.id_str());
                            self.store.store_state_channel(new_sc)?;
                            self.send_packet_offers(logger).await
                        }
                        Err(Error::StateChannel(err)) => match *err {
                            StateChannelError::CausalConflict { sc, conflicts_with } => self
                                .store
                                .store_conflicting_state_channel(sc, conflicts_with),
                            err => {
                                info!(logger, "ignoring banner: {:?}", err);
                                Ok(())
                            }
                        },
                        Err(err) => {
                            info!(logger, "ignoring banner: {:?}", err);
                            Ok(())
                        }
                    }
                } else {
                    Ok(())
                }
            }
            Msg::Reject(_) => {
                debug!(logger, "dropping rejected packet");
                let _ = self.store.deque_packet();
                Ok(())
            }
        }
    }

    async fn handle_downlink(&mut self, logger: &Logger, packet: &helium_proto::Packet) {
        match self.downlinks.send(packet.clone().into()).await {
            Ok(()) => (),
            Err(_) => {
                warn!(logger, "failed to push downlink")
            }
        }
    }

    async fn send_packet_offers(&mut self, logger: &Logger) -> Result {
        if self.state_channel.capacity() == 0 {
            return Ok(());
        }
        while let Some(packet) = self.store.pop_waiting_packet() {
            self.send_offer(logger, &packet).await?;
            self.store.que_packet(packet)?;
            if self.state_channel.capacity() == 0 {
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

    async fn send_packet(&mut self, _logger: &Logger, packet: Option<&QuePacket>) -> Result {
        if packet.is_none() {
            return Ok(());
        }
        let packet = packet.unwrap();
        match StateChannelMessage::packet(
            packet.packet().clone(),
            &self.keypair,
            self.region.clone(),
            packet.hold_time().as_millis() as u64,
        ) {
            Ok(message) => Ok(self.state_channel.send(message.to_message()).await?),
            Err(err) => Err(err),
        }
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
