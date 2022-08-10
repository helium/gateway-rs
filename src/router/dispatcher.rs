use crate::{
    gateway,
    router::{self, RouterClient},
    service::{self, gateway::GatewayService},
    sync, CacheSettings, Error, KeyedUri, Keypair, Packet, Region, RegionParams, Result, Settings,
};
use exponential_backoff::Backoff;
use futures::{
    task::{Context, Poll},
    TryFutureExt,
};
use helium_proto::BlockchainVarV1;
use slog::{info, o, warn, Logger};
use slog_scope;
use std::{
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{task::JoinHandle, time};
use tokio_stream::{self, StreamExt, StreamMap};

#[derive(Debug)]
pub enum Message {
    Uplink {
        packet: Packet,
        received_time: Instant,
    },
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
    pub gateway_version: u64,
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

    pub async fn uplink(&self, packet: Packet, received_time: Instant) -> Result {
        self.0
            .send(Message::Uplink {
                packet,
                received_time,
            })
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
    seed_gateways: Vec<KeyedUri>,
    routing_height: u64,
    region_height: u64,
    cache_settings: CacheSettings,
    gateway_retry: u32,
    routers: Vec<RouterEntry>,
    default_routers: Option<Vec<KeyedUri>>,
}

#[derive(PartialEq, Eq, Hash)]
struct RouterKey {
    oui: u32,
    uri: KeyedUri,
}

#[derive(Debug)]
struct RouterEntry {
    dispatch: router::client::MessageSender,
    join_handle: JoinHandle<Result>,
}

const GATEWAY_BACKOFF_RETRIES: u32 = 10;
const GATEWAY_BACKOFF_MIN_WAIT: Duration = Duration::from_secs(5);
const GATEWAY_BACKOFF_MAX_WAIT: Duration = Duration::from_secs(1800); // 30 minutes

const GATEWAY_CHECK_INTERVAL: Duration = Duration::from_secs(900); // 15 minutes
const GATEWAY_MAX_BLOCK_AGE: Duration = Duration::from_secs(1800); // 30 minutes

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
enum GatewayStream {
    Routing,
    RegionParams,
}

type GatewayStreams = StreamMap<GatewayStream, service::gateway::Streaming>;

impl Dispatcher {
    // Allow mutable key type for HashMap with Uri in the key
    #[allow(clippy::mutable_key_type)]
    pub fn new(
        messages: MessageReceiver,
        downlinks: gateway::MessageSender,
        settings: &Settings,
    ) -> Result<Self> {
        let seed_gateways = settings.gateways.clone();
        let default_routers = settings.routers.clone();
        let cache_settings = settings.cache.clone();
        Ok(Self {
            keypair: settings.keypair.clone(),
            region: settings.region,
            messages,
            downlinks,
            seed_gateways,
            routers: vec![],
            routing_height: 0,
            region_height: 0,
            default_routers,
            cache_settings,
            gateway_retry: 0,
        })
    }

    pub async fn run(&mut self, shutdown: triggered::Listener, logger: &Logger) -> Result {
        let logger = logger.new(o!("module" => "dispatcher"));
        info!(logger, "starting"; 
            "region" => self.region);

        if let Some(default_routers) = &self.default_routers {
            for default_router in default_routers {
                let router_entry = self
                    .start_router(shutdown.clone(), default_router.clone())
                    .await?;

                self.routers.push(router_entry);
                info!(logger, "default router";
                    "pubkey" => default_router.pubkey.to_string(),
                    "uri" => default_router.uri.to_string());
            }
        }

        let gateway_backoff = Backoff::new(
            GATEWAY_BACKOFF_RETRIES,
            GATEWAY_BACKOFF_MIN_WAIT,
            GATEWAY_BACKOFF_MAX_WAIT,
        );
        loop {
            if shutdown.is_triggered() {
                // Prevent unneeded seed reselection
                return Ok(());
            }
            // Select seed
            let seed_gateway = GatewayService::select_seed(&self.seed_gateways)?;
            info!(logger, "seed gateway";
                "pubkey" => seed_gateway.uri.pubkey.to_string(),
                "uri" => seed_gateway.uri.uri.to_string());

            tokio::select! {
                    _ = shutdown.clone() => {
                        info!(logger, "shutting down");
                        return Ok(())
                    },
                // Try to select a random validator from the seed and fetch the needed streams
                gateway = Self::select_gateway(seed_gateway, &shutdown, &logger)
                    .and_then(|service | self.setup_gateway_streams(service, &logger))
                     => match gateway {
                        Ok(Some((service, gateway_streams, default_region_params))) => {
                            self.downlinks.region_params_changed(default_region_params).await;
                            self.run_with_gateway(service, gateway_streams,  shutdown.clone(), &logger)
                                .await?;
                            },
                        Ok(None) =>
                            return Ok(()),
                        Err(_err) => ()
                    }
            }

            self.prepare_gateway_change(&gateway_backoff, shutdown.clone(), &logger)
                .await;
        }
    }

    async fn select_gateway(
        mut seed_gateway: GatewayService,
        shutdown: &triggered::Listener,
        logger: &Logger,
    ) -> Result<Option<GatewayService>> {
        match seed_gateway.random_new(5, shutdown.clone()).await {
            Ok(result) => Ok(result),
            Err(err) => {
                warn!(logger, "gateway selection error: {err:?}";
                    "pubkey" => seed_gateway.uri.pubkey.to_string(),
                    "uri" => seed_gateway.uri.uri.to_string());
                Err(err)
            }
        }
    }

    async fn setup_gateway_streams(
        &mut self,
        gateway: Option<GatewayService>,
        logger: &Logger,
    ) -> Result<Option<(GatewayService, GatewayStreams, RegionParams)>> {
        if gateway.is_none() {
            return Ok(None);
        }
        let mut gateway = gateway.unwrap();
        let mut routing_gateway = gateway.clone();
        let routing = routing_gateway.routing(self.routing_height);
        let default_region_params = gateway
            .region_params_for(&self.region, self.keypair.clone())
            .await?;
        let region_params = gateway.region_params(self.keypair.clone());
        match tokio::try_join!(routing, region_params) {
            Ok((routing, region_params)) => {
                let stream_map = StreamMap::from_iter([
                    (GatewayStream::Routing, routing),
                    (GatewayStream::RegionParams, region_params),
                ]);
                Ok(Some((gateway, stream_map, default_region_params)))
            }
            Err(err) => {
                warn!(logger, "gateway stream setup error: {err:?} "; 
                    "pubkey" => gateway.uri.pubkey.to_string(),
                    "uri" => gateway.uri.uri.to_string());
                Err(err)
            }
        }
    }

    async fn run_with_gateway(
        &mut self,
        mut gateway: GatewayService,
        mut streams: GatewayStreams,
        shutdown: triggered::Listener,
        logger: &Logger,
    ) -> Result {
        info!(logger, "using gateway";
            "pubkey" => gateway.uri.pubkey.to_string(),
            "uri" => gateway.uri.uri.to_string());

        // Initialize liveness check for gateway
        let mut gateway_check = time::interval(GATEWAY_CHECK_INTERVAL);
        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                gateway_message = streams.next() => match gateway_message {
                    Some((gateway_stream, Ok(gateway_message))) => match gateway_stream {
                        GatewayStream::Routing => info!(logger, "ignoring routing update"),
                        GatewayStream::RegionParams => self.handle_region_params_update(&gateway_message, logger).await,
                    },
                    Some((gateway_stream, Err(err))) =>  {
                        match gateway_stream {
                            GatewayStream::Routing =>  warn!(logger, "gateway routing stream error: {err:?}"),
                            GatewayStream::RegionParams =>  warn!(logger, "gateway region_params stream error: {err:?}"),
                        }
                        return Ok(())
                    },
                    None => {
                        warn!(logger, "gateway streams closed");
                        return Ok(());
                }
                },
                _ = gateway_check.tick() => match self.check_gateway(&mut gateway, logger).await {
                    Ok(()) => {
                        self.gateway_retry = 0
                    },
                    Err(err) => {
                        warn!(logger, "gateway check error: {err}");
                        return Ok(())
                    }
                },
                message = self.messages.recv() => match message {
                    Some(message) => self.handle_message(message, Some(&mut gateway.clone()), logger).await,
                    None => {
                        warn!(logger, "messages channel closed");
                        return Ok(())
                    }
                }
            }
        }
    }

    async fn check_gateway(&mut self, gateway: &mut GatewayService, logger: &Logger) -> Result {
        let (_, block_age) = gateway.height().await?;
        info!(logger, "checking gateway"; 
            "pubkey" => gateway.uri.pubkey.to_string(),
            "block_age" => block_age);
        if block_age > GATEWAY_MAX_BLOCK_AGE.as_secs() {
            return Err(Error::gateway_service_check(
                block_age,
                GATEWAY_MAX_BLOCK_AGE.as_secs(),
            ));
        }
        Ok(())
    }

    async fn prepare_gateway_change(
        &mut self,
        backoff: &Backoff,
        shutdown: triggered::Listener,
        logger: &Logger,
    ) {
        // Check if shutdown trigger already happened
        if shutdown.is_triggered() {
            return;
        }

        // Reset routing and region heigth for the next gateway
        self.routing_height = 0;
        self.region_height = 0;

        // Use backof to sleep exponentially longer
        self.gateway_retry += 1;
        let sleep = backoff
            .next(self.gateway_retry)
            .unwrap_or(GATEWAY_BACKOFF_MAX_WAIT);

        // Select over either shutdown or sleep, and handle messages that don't
        // require a gateway
        info!(logger, "selecting new gateway in {}s", sleep.as_secs());
        tokio::select! {
            _ = shutdown => {},
            _ = time::sleep(sleep) => {}
            message = self.messages.recv() => match message {
                Some(message) => self.handle_message(message, None, logger).await,
                None => warn!(logger, "ignoring closed messages channel"),
            }
        }
    }

    async fn handle_message(
        &self,
        message: Message,
        gateway: Option<&mut GatewayService>,
        logger: &Logger,
    ) {
        match message {
            Message::Uplink {
                packet,
                received_time,
            } => self.handle_uplink(&packet, received_time, logger).await,
            Message::Config { keys, response } => {
                let reply = if let Some(gateway) = gateway {
                    gateway.config(keys).await
                } else {
                    Err(Error::no_service())
                };
                response.send(reply, logger)
            }
            Message::Height { response } => {
                let reply = if let Some(gateway) = gateway {
                    let gateway_version = gateway.version().await.unwrap_or(0);
                    gateway
                        .height()
                        .await
                        .map(|(height, block_age)| HeightResponse {
                            gateway: gateway.uri.clone(),
                            height,
                            block_age,
                            gateway_version,
                        })
                } else {
                    Err(Error::no_service())
                };
                response.send(reply, logger)
            }
            Message::Region { response } => response.send(Ok(self.region), logger),
        }
    }

    async fn handle_uplink(&self, packet: &Packet, received: Instant, logger: &Logger) {
        for router_entry in self.routers.iter() {
            match router_entry.dispatch.uplink(packet.clone(), received).await {
                Ok(()) => (),
                Err(err) => warn!(logger, "ignoring router dispatch error: {err:?}"),
            }
        }
    }

    async fn handle_region_params_update<R: service::gateway::Response>(
        &mut self,
        response: &R,
        logger: &Logger,
    ) {
        let update_height = response.height();
        let current_height = self.region_height;
        if update_height <= self.region_height {
            warn!(
                logger,
                "region_params returned invalid height {update_height} while at {current_height}"
            );
            return;
        }
        match response.region_params() {
            Ok(region_params) => {
                self.region_height = update_height;
                self.region = region_params.region;
                info!(
                    logger, "updated region";
                    "region" => self.region,
                    "height" => update_height
                );
                // Tell downlink handler
                self.downlinks
                    .region_params_changed(region_params.clone())
                    .await;
                // Tell routers about it

                for router_entry in self.routers.iter() {
                    router_entry.dispatch.region_changed(self.region).await;
                }
            }
            Err(err) => {
                warn!(logger, "error decoding region: {err:?}");
            }
        }
    }

    async fn start_router(
        &self,
        shutdown: triggered::Listener,

        uri: KeyedUri,
    ) -> Result<RouterEntry> {
        // We start the router scope at the root logger to avoid picking up the
        // previously set KV pairs (which causes dupes)
        let logger = slog_scope::logger();
        let (client_tx, client_rx) = router::client::message_channel(10);
        let mut client = RouterClient::new(
            self.region,
            uri,
            self.downlinks.clone(),
            self.keypair.clone(),
            self.cache_settings.clone(),
        )
        .await?;
        let join_handle =
            tokio::spawn(async move { client.run(client_rx, shutdown, &logger).await });
        Ok(RouterEntry {
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
