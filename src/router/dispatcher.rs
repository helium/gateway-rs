use crate::{
    gateway,
    router::{self, RouterClient, Routing},
    service::{self, gateway::GatewayService},
    sync, CacheSettings, Error, KeyedUri, Keypair, Packet, Region, Result, Settings,
};
use futures::{
    future::join_all,
    task::{Context, Poll},
    TryFutureExt,
};
use helium_proto::BlockchainVarV1;
use http::uri::Uri;
use slog::{debug, info, o, warn, Logger};
use slog_scope;
use std::{collections::HashMap, pin::Pin, sync::Arc, time::Duration};
use tokio::{task::JoinHandle, time};
use tokio_stream::{self as stream, StreamExt};

#[derive(Debug)]
pub enum Message {
    Uplink(Packet),
    Config {
        keys: Vec<String>,
        response: sync::ResponseSender<Result<Vec<BlockchainVarV1>>>,
    },
    Height {
        response: sync::ResponseSender<Result<HeightResponse>>,
    },
    Region {
        response: sync::ResponseSender<Result<Region>>,
    },
}

#[derive(Debug)]
pub struct HeightResponse {
    pub gateway: KeyedUri,
    pub height: u64,
    pub block_age: u64,
}

pub type MessageSender = sync::MessageSender<Message>;
pub type MessageReceiver = sync::MessageReceiver<Message>;

pub fn message_channel(size: usize) -> (MessageSender, MessageReceiver) {
    sync::message_channel(size)
}

impl MessageSender {
    pub async fn config(&self, keys: &[String]) -> Result<Vec<BlockchainVarV1>> {
        let (tx, rx) = sync::response_channel();
        let _ = self
            .0
            .send(Message::Config {
                keys: keys.to_vec(),
                response: tx,
            })
            .await;
        rx.recv().await?
    }

    pub async fn uplink(&self, packet: Packet) -> Result {
        self.0
            .send(Message::Uplink(packet))
            .map_err(|_| Error::channel())
            .await
    }

    pub async fn height(&self) -> Result<HeightResponse> {
        let (tx, rx) = sync::response_channel();
        let _ = self.0.send(Message::Height { response: tx }).await;
        rx.recv().await?
    }

    pub async fn region(&self) -> Result<Region> {
        let (tx, rx) = sync::response_channel();
        let _ = self.0.send(Message::Region { response: tx }).await;
        rx.recv().await?
    }
}

pub struct Dispatcher {
    keypair: Arc<Keypair>,
    region: Region,
    messages: MessageReceiver,
    downlinks: gateway::MessageSender,
    gateways: Vec<KeyedUri>,
    routing_height: u64,
    region_height: u64,
    default_router: KeyedUri,
    cache_settings: CacheSettings,
    routers: HashMap<RouterKey, RouterEntry>,
}

#[derive(PartialEq, Eq, Hash)]
struct RouterKey {
    oui: u32,
    uri: Uri,
}

#[derive(Debug)]
struct RouterEntry {
    routing: Routing,
    dispatch: router::client::MessageSender,
    join_handle: JoinHandle<Result>,
}

impl Dispatcher {
    // Allow mutable key type for HashMap with Uri in the key
    #[allow(clippy::mutable_key_type)]
    pub fn new(
        messages: MessageReceiver,
        downlinks: gateway::MessageSender,
        settings: &Settings,
    ) -> Result<Self> {
        let gateways = settings.gateways.clone();
        let routers = HashMap::with_capacity(5);
        let default_router = settings.default_router().clone();
        let cache_settings = settings.cache.clone();
        Ok(Self {
            keypair: settings.keypair.clone(),
            region: settings.region,
            messages,
            downlinks,
            gateways,
            routers,
            routing_height: 0,
            region_height: 0,
            default_router,
            cache_settings,
        })
    }

    pub async fn run(&mut self, shutdown: triggered::Listener, logger: &Logger) -> Result {
        let logger = logger.new(o!("module" => "dispatcher"));
        info!(logger, "starting"; 
            "region" => self.region.to_string());

        info!(logger, "default router";
            "pubkey" => self.default_router.pubkey.to_string(),
            "uri" => self.default_router.uri.to_string());

        loop {
            let gateway = GatewayService::random_new(&self.gateways)?;
            info!(logger, "using gateway";
                "pubkey" => gateway.uri.pubkey.to_string(),
                "uri" => gateway.uri.uri.to_string());
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                gateway_streams = Self::gateway_streams(gateway.clone(), self.routing_height, self.keypair.clone()) => {
                    match gateway_streams {
                        Ok((routing_stream, region_stream)) => self.run_with_gateway_streams(gateway, routing_stream, region_stream, shutdown.clone(), &logger).await?,
                        Err(err) => warn!(logger, "gateway error: {err:?}")
                    }
                    // Check if trigger happened in run_with_routing_stream
                    if shutdown.is_triggered() {
                        return Ok(())
                    } else {
                        // Wait a bit before trying another gateway service
                        self.handle_gateway_change(&logger).await;
                        time::sleep(Duration::from_secs(5)).await;
                    }
                },
                message = self.messages.recv() => match message {
                    Some(message) => self.handle_message(message, &mut gateway.clone(), &logger).await,
                    None => warn!(logger, "ignoring closed messages channel"),
                }
            }
        }
    }

    async fn gateway_streams(
        mut gateway: GatewayService,
        routing_height: u64,
        keypair: Arc<Keypair>,
    ) -> Result<(service::gateway::Streaming, service::gateway::Streaming)> {
        let mut routing_gateway = gateway.clone();
        let routing = routing_gateway.routing(routing_height);
        let region_params = gateway.region_params(keypair);
        tokio::try_join!(routing, region_params)
    }

    async fn run_with_gateway_streams(
        &mut self,
        mut gateway: GatewayService,
        mut routing_stream: service::gateway::Streaming,
        mut region_stream: service::gateway::Streaming,
        shutdown: triggered::Listener,
        logger: &Logger,
    ) -> Result {
        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    // Wait for all dispatched routers to shut down
                    let _ = join_all(self.routers.values_mut());
                    return Ok(())
                },
                routing = routing_stream.message() => match routing {
                    Ok(Some(response)) => self.handle_routing_update(&mut gateway, &response, &shutdown, logger).await,
                    Ok(None) => {
                        warn!(logger, "gateway routing stream closed");
                        return Ok(());
                    },
                    Err(err) => {
                        warn!(logger, "gateway routing stream error: {err:?}");
                        return Ok(())
                    },
                },
                region = region_stream.message() => match region {
                    Ok(Some(response)) => self.handle_region_update(&response, logger).await,
                    Ok(None) => {
                        warn!(logger, "gateway region stream closed");
                        return Ok(());
                    },
                    Err(err) => {
                        warn!(logger, "gateway region stream error: {err:?}");
                        return Ok(())
                    },
                },
                message = self.messages.recv() => match message {
                    Some(message) => self.handle_message(message, &mut gateway, logger).await,
                    None => warn!(logger, "ignoring closed messages channel"),
                }
            }
        }
    }

    async fn handle_gateway_change(&mut self, _logger: &Logger) {
        for (_, router_entry) in self.routers.drain() {
            router_entry.dispatch.gateway_changed().await;
        }
        // Reset routing and region heigth for the next gateway
        self.routing_height = 0;
        self.region_height = 0;
    }

    async fn handle_message(
        &self,
        message: Message,
        gateway: &mut GatewayService,
        logger: &Logger,
    ) {
        match message {
            Message::Uplink(packet) => self.handle_uplink(&packet, logger).await,
            Message::Config { keys, response } => response.send(gateway.config(keys).await, logger),
            Message::Height { response } => {
                let reply = gateway
                    .height()
                    .await
                    .map(|(height, block_age)| HeightResponse {
                        gateway: gateway.uri.clone(),
                        height,
                        block_age,
                    });
                response.send(reply, logger)
            }
            Message::Region { response } => response.send(Ok(self.region), logger),
        }
    }

    async fn handle_uplink(&self, packet: &Packet, logger: &Logger) {
        let mut handled = false;
        for router_entry in self.routers.values() {
            if router_entry.routing.matches_routing_info(packet.routing()) {
                match router_entry.dispatch.uplink(packet.clone()).await {
                    Ok(()) => (),
                    Err(err) => warn!(logger, "ignoring router dispatch error: {err:?}"),
                }
                handled = true;
            }
        }
        if !handled {
            for (router_key, router_entry) in &self.routers {
                if router_key.uri == self.default_router.uri {
                    debug!(logger, "sending to default router");
                    let _ = router_entry.dispatch.uplink(packet.clone()).await;
                }
            }
        }
    }

    async fn handle_region_update(
        &mut self,
        response: &service::gateway::Response,
        logger: &Logger,
    ) {
        let update_height = response.height();
        let current_height = self.region_height;
        if update_height <= self.region_height {
            warn!(
                logger,
                "region returned invalid height {update_height} while at {current_height}"
            );
            return;
        }
        match response.region() {
            Ok(region) => {
                self.region_height = update_height;
                self.region = region;
                info!(
                    logger,
                    "updated region to {region} at height {update_height}"
                )
            }
            Err(err) => {
                warn!(logger, "error decoding region: {err:?}");
            }
        }
    }

    async fn handle_routing_update(
        &mut self,
        gateway: &mut GatewayService,
        response: &service::gateway::Response,
        shutdown: &triggered::Listener,
        logger: &Logger,
    ) {
        let update_height = response.height();
        let current_height = self.routing_height;
        if update_height <= self.routing_height {
            warn!(
                logger,
                "routing returned invalid height {update_height} while at {current_height}",
            );
            return;
        }
        let routing_protos = match response.routings() {
            Ok(v) => v,
            Err(err) => {
                warn!(logger, "error decoding routing {err:?}");
                return;
            }
        };
        let mut proto_stream = stream::iter(routing_protos.iter());
        while let Some(proto) = proto_stream.next().await {
            match Routing::from_proto(logger, proto) {
                Ok(routing) => {
                    self.handle_oui_routing_update(gateway, &routing, shutdown, logger)
                        .await
                }
                Err(err) => warn!(logger, "failed to parse routing: {err:?}"),
            }
        }
        self.routing_height = update_height;
        info!(logger, "updated routing to height {:?}", update_height)
    }

    #[allow(clippy::map_entry)]
    async fn handle_oui_routing_update(
        &mut self,
        gateway: &mut GatewayService,
        routing: &Routing,
        shutdown: &triggered::Listener,
        logger: &Logger,
    ) {
        let mut uris = stream::iter(routing.uris.iter());
        while let Some(uri) = uris.next().await {
            let key = RouterKey {
                oui: routing.oui,
                uri: uri.uri.clone(),
            };
            // We have to allow clippy::map_entry above since we need to borrow
            // immutable before borrowing as mutable to insert
            if !self.routers.contains_key(&key) {
                match self
                    .start_router(gateway, shutdown.clone(), routing.clone(), uri.clone())
                    .await
                {
                    Ok(router_entry) => {
                        self.routers.insert(key, router_entry);
                    }
                    Err(err) => {
                        warn!(logger, "faild to construct router: {err:?}");
                    }
                }
            }
        }
        // Remove any routers that are not in the new oui uri list
        self.routers.retain(|key, entry| {
            if key.oui == routing.oui && !entry.routing.contains_uri(&key.uri) {
                // Router will be removed from the map. The router is expected
                // to stop itself when it receives the routing message
                info!(logger, "removing router";
                    "oui" => key.oui,
                    "uri" => key.uri.to_string()
                );
                return false;
            }
            true
        });
    }

    async fn start_router(
        &self,
        gateway: &mut GatewayService,
        shutdown: triggered::Listener,
        routing: Routing,
        uri: KeyedUri,
    ) -> Result<RouterEntry> {
        // We start the router scope at the root logger to avoid picking up the
        // previously set KV pairs (which causes dupes)
        let logger = slog_scope::logger();
        let (client_tx, client_rx) = router::client::message_channel(10);
        let mut client = RouterClient::new(
            routing.oui,
            self.region,
            uri,
            gateway.clone(),
            self.downlinks.clone(),
            self.keypair.clone(),
            self.cache_settings.clone(),
        )
        .await?;
        let join_handle =
            tokio::spawn(async move { client.run(client_rx, shutdown, &logger).await });
        Ok(RouterEntry {
            routing,
            dispatch: client_tx,
            join_handle,
        })
    }
}

impl std::future::Future for RouterEntry {
    type Output = std::result::Result<Result, tokio::task::JoinError>;

    fn poll(
        mut self: Pin<&mut Self>,
        cxt: &mut Context<'_>,
    ) -> Poll<<Self as futures::Future>::Output> {
        Pin::new(&mut self.join_handle).poll(cxt)
    }
}
