use crate::{
    error::{Error, StateChannelError},
    router::{Dispatch, QuePacket, RouterStore},
    service::gateway::GatewayService,
    service::router::{Service as RouterService, StateChannelService},
    CacheSettings, KeyedUri, Keypair, Packet, Region, Result, StateChannel, StateChannelCausality,
    StateChannelKey, StateChannelMessage,
};
use helium_proto::{blockchain_state_channel_message_v1::Msg, BlockchainStateChannelV1};
use slog::{info, o, warn, Logger};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct RouterClient {
    client: RouterService,
    oui: u32,
    region: Region,
    keypair: Arc<Keypair>,
    downlinks: mpsc::Sender<Packet>,
    gateway: GatewayService,
    store: RouterStore,
    state_channel: StateChannelService,
}

impl RouterClient {
    pub async fn new(
        oui: u32,
        region: Region,
        uri: KeyedUri,
        gateway: GatewayService,
        downlinks: mpsc::Sender<Packet>,
        keypair: Arc<Keypair>,
        settings: CacheSettings,
    ) -> Result<Self> {
        let mut client = RouterService::new(uri)?;
        let state_channel = client.state_channel()?;
        let store = RouterStore::new(&settings);
        Ok(Self {
            client,
            oui,
            region,
            keypair,
            downlinks,
            store,
            state_channel,
            gateway,
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
            "public_key" => self.client.uri.public_key.to_string(),
            "uri" => self.client.uri.uri.to_string(),
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
                sc_message = self.state_channel.message() =>  match sc_message {
                    Ok(Some(message)) => {
                        if let Some(inner_msg) = message.msg {
                        match self.handle_state_channel_message(&logger, inner_msg.into()).await {
                            Ok(()) => (),
                            Err(err) => warn!(logger, "state channel handling error {:?}", err),
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
        if self.store.state_channel_count() == 0 {
            self.state_channel.connect().await?;
        }
        self.send_packet(logger, Some(&QuePacket::from(uplink)))
            .await
        // self.store.store_waiting_packet(uplink)?;
        // if self.store.state_channel_count().await? == 0 {
        //     // No banner received yet, start connect
        //     return self.state_channel.connect().await;
        // }
        // self.send_packet_offers(logger).await
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
                let purchase_sc = self
                    .mk_state_channel(purchase.sc.to_owned(), |known_sc, new_sc| {
                        if let Some(known_sc) = known_sc {
                            return known_sc.is_valid_purchase(new_sc, packet.as_ref());
                        }
                        Ok(())
                    })
                    .await?;
                info!(logger, "received purchase";
                    "sc_id" => purchase_sc.id_key());
                self.send_packet(logger, packet.as_ref()).await
            }
            Msg::Banner(banner) => {
                let banner_sc = self
                    .mk_state_channel(banner.sc.to_owned(), |_, _| Ok(()))
                    .await?;
                info!(logger, "received banner";
                    "sc_id" => banner_sc.id_key());
                self.send_packet_offers(logger).await
            }
            Msg::Reject(_) => {
                let _ = self.store.deque_packet();
                Ok(())
            }
        }
    }

    async fn mk_state_channel<F>(
        &mut self,
        sc: Option<BlockchainStateChannelV1>,
        final_validation: F,
    ) -> Result<StateChannel>
    where
        F: Fn(Option<&StateChannel>, &StateChannel) -> Result,
    {
        if sc.is_none() {
            return Err(StateChannelError::not_found());
        }
        let sc = sc.unwrap();
        // Check if we already have a stored state channel with the given key
        // and accept it without checking is active or validating
        let public_key = self.keypair.public_key();
        if let Some(known_sc) = self.store.get_state_channel(&sc.id)? {
            if sc.id == known_sc.id() {
                let sc = known_sc.with_sc(sc)?;
                final_validation(Some(known_sc), &sc)?;
                self.store.store_state_channel(sc.clone())?;
                Ok(sc)
            } else {
                // the new sc has a different id
                let sc = StateChannel::from_sc(sc, &mut self.gateway).await?;
                match known_sc.is_valid_sc_for(public_key, &sc) {
                    Ok(causality) => match final_validation(Some(known_sc), &sc) {
                        Ok(()) => {
                            // Ensure the new sc newer than the last known one.
                            // We only check for this gateway rather than the
                            // whole state channel to save some time
                            if causality == StateChannelCausality::Cause {
                                self.store.store_state_channel(sc.clone())?;
                                Ok(sc)
                            } else {
                                Ok(known_sc.clone())
                            }
                        }
                        Err(err) => Err(err),
                    },
                    Err(Error::StateChannel(err)) => {
                        // Mark the state channel as conflicting and store the
                        // one that maximizes the number of dcs for this gateway
                        let max_return_sc =
                            if known_sc.num_dcs_for(public_key) > sc.num_dcs_for(public_key) {
                                known_sc.clone()
                            } else {
                                sc
                            };
                        self.store.store_conflicting_state_channel(max_return_sc)?;
                        Err(Error::StateChannel(err))
                    }
                    Err(err) => Err(err),
                }
            }
        } else {
            // No previously known sc with that id
            let sc = StateChannel::from_sc(sc, &mut self.gateway).await?;
            match sc.is_valid_for(self.keypair.public_key()) {
                Ok(()) => match final_validation(None, &sc) {
                    Ok(()) => {
                        self.store.store_state_channel(sc.clone())?;
                        Ok(sc)
                    }
                    Err(err) => Err(err),
                },
                Err(Error::StateChannel(err)) => {
                    self.store.store_conflicting_state_channel(sc)?;
                    Err(Error::StateChannel(err))
                }
                Err(err) => Err(err),
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
