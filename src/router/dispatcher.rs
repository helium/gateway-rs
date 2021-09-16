use super::{RouterClient, Routing};
use crate::{
    service::gateway::{self, GatewayService},
    CacheSettings, KeyedUri, Keypair, Packet, Region, Result, Settings,
};
use futures::{
    future::join_all,
    task::{Context, Poll},
};
use http::uri::Uri;
use slog::{debug, info, o, warn, Logger};
use slog_scope;
use std::{collections::HashMap, pin::Pin, sync::Arc, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle, time};
use tokio_stream::{self as stream, StreamExt};

#[derive(Debug, Clone)]
pub enum Dispatch {
    Packet(Packet),
    Gateway(GatewayService),
}

pub struct Dispatcher {
    keypair: Arc<Keypair>,
    region: Region,
    downlinks: mpsc::Sender<Packet>,
    uplinks: mpsc::Receiver<Packet>,
    gateways: Vec<KeyedUri>,
    routing_height: u64,
    gateway: GatewayService,
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
    dispatch: mpsc::Sender<Dispatch>,
    join_handle: JoinHandle<Result>,
}

impl Dispatcher {
    // Allow mutable key type for HashMap with Uri in the key
    #[allow(clippy::mutable_key_type)]
    pub fn new(
        downlinks: mpsc::Sender<Packet>,
        uplinks: mpsc::Receiver<Packet>,
        settings: &Settings,
    ) -> Result<Self> {
        let gateways = settings.gateways.clone();
        let routers = HashMap::with_capacity(5);
        let default_router = settings.default_router().clone();
        let cache_settings = settings.cache.clone();
        let gateway = GatewayService::random_new(&gateways)?;
        Ok(Self {
            keypair: settings.keypair.clone(),
            region: settings.region.clone(),
            uplinks,
            downlinks,
            gateways,
            gateway,
            routers,
            routing_height: 0,
            default_router,
            cache_settings,
        })
    }

    pub async fn run(&mut self, shutdown: triggered::Listener, logger: &Logger) -> Result {
        let logger = logger.new(o!("module" => "dispatcher"));
        info!(logger, "starting");

        info!(logger, "default router";
            "public_key" => self.default_router.public_key.to_string(),
            "uri" => self.default_router.uri.to_string());

        loop {
            info!(logger, "using gateway";
                "public_key" => self.gateway.uri.public_key.to_string(),
                "uri" => self.gateway.uri.uri.to_string());
            tokio::select! {
                    _ = shutdown.clone() => {
                        info!(logger, "shutting down");
                        return Ok(())
                    },
                    routing_stream = self.gateway.routing(self.routing_height) => {
                        match routing_stream {
                            Ok(stream) => self.run_with_routing_stream(stream, shutdown.clone(), &logger).await?,
                            Err(err) => warn!(logger, "gateway error: {:?}", err)
                        }
                        // Check if trigger happened in run_with_routing_stream
                        if shutdown.is_triggered() {
                            return Ok(())
                        } else {
                            // Wait a bit before trying another gateway service
                            time::sleep(Duration::from_secs(5)).await;
                        }
                    }
            }
            let gateway = GatewayService::random_new(&self.gateways)?;
            self.handle_gateway_change(gateway, &logger).await;
        }
    }

    async fn run_with_routing_stream(
        &mut self,
        mut routing_stream: gateway::Streaming,
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
                    Ok(Some(response)) => self.handle_routing_update(&response, &shutdown, logger).await,
                    Ok(None) => {return Ok(())},
                    Err(err) => {
                        info!(logger, "gateway error: {:?}", err);
                        return Ok(())
                    }
                },
                uplink = self.uplinks.recv() => match uplink {
                    Some(packet) => self.handle_uplink(&packet, logger).await,
                    None => warn!(logger, "ignoring closed uplinks channel"),
                },
            }
        }
    }

    async fn handle_gateway_change(&mut self, gateway: GatewayService, _logger: &Logger) {
        self.gateway = gateway.to_owned();
        for router_entry in self.routers.values() {
            let _ = router_entry
                .dispatch
                .send(Dispatch::Gateway(gateway.clone()))
                .await;
        }
    }

    async fn handle_uplink(&self, packet: &Packet, logger: &Logger) {
        let mut handled = false;
        for router_entry in self.routers.values() {
            if router_entry.routing.matches_routing_info(packet.routing()) {
                match router_entry
                    .dispatch
                    .send(Dispatch::Packet(packet.clone()))
                    .await
                {
                    Ok(()) => (),
                    Err(_) => warn!(logger, "ignoring router dispatch error"),
                }
                handled = true;
            }
        }
        if !handled {
            for (router_key, router_entry) in &self.routers {
                if router_key.uri == self.default_router.uri {
                    debug!(logger, "sending to default router");
                    let _ = router_entry
                        .dispatch
                        .send(Dispatch::Packet(packet.clone()))
                        .await;
                }
            }
        }
    }

    async fn handle_routing_update(
        &mut self,
        response: &gateway::Response,
        shutdown: &triggered::Listener,
        logger: &Logger,
    ) {
        let update_height = response.height();
        if update_height <= self.routing_height {
            warn!(
                logger,
                "router returned invalid height {:?} while at {:?}",
                update_height,
                self.routing_height
            );
            return;
        }
        let routing_protos = match response.routings() {
            Ok(v) => v,
            Err(err) => {
                warn!(logger, "error decoding routing {:?}", err);
                return;
            }
        };
        let mut proto_stream = stream::iter(routing_protos.iter());
        while let Some(proto) = proto_stream.next().await {
            match Routing::from_proto(logger, proto) {
                Ok(routing) => {
                    self.handle_oui_routing_update(&routing, shutdown, logger)
                        .await
                }
                Err(err) => warn!(logger, "failed to parse routing: {:?}", err),
            }
        }
        self.routing_height = update_height;
        info!(
            logger,
            "updated routing to height {:?}", self.routing_height
        )
    }

    #[allow(clippy::map_entry)]
    async fn handle_oui_routing_update(
        &mut self,
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
                    .start_router(shutdown.clone(), routing.clone(), uri.clone())
                    .await
                {
                    Ok(router_entry) => {
                        self.routers.insert(key, router_entry);
                    }
                    Err(err) => {
                        warn!(logger, "faild to construct router: {:?}", err);
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
        shutdown: triggered::Listener,
        routing: Routing,
        uri: KeyedUri,
    ) -> Result<RouterEntry> {
        // We start the router scope at the root logger to avoid picking up the
        // previously set KV pairs (which causes dupes)
        let logger = slog_scope::logger();
        let (dispatch, dispatch_receiver) = mpsc::channel(10);
        let mut client = RouterClient::new(
            routing.oui,
            self.region.clone(),
            uri,
            self.gateway.clone(),
            self.downlinks.clone(),
            self.keypair.clone(),
            self.cache_settings.clone(),
        )
        .await?;
        let join_handle =
            tokio::spawn(async move { client.run(dispatch_receiver, shutdown, &logger).await });
        Ok(RouterEntry {
            routing,
            dispatch,
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
