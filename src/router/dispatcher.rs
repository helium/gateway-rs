use super::{RouterClient, Routing};
use crate::{
    service::{gateway, router},
    KeyedUri, Keypair, LinkPacket, Region, Result, Settings,
};
use futures::future::join_all;
use http::uri::Uri;
use slog::{debug, info, o, warn, Logger};
use slog_scope;
use std::{
    collections::{hash_map, HashMap},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
    time,
};

pub struct Dispatcher {
    keypair: Arc<Keypair>,
    region: Region,
    shutdown: triggered::Listener,
    downlinks: mpsc::Sender<LinkPacket>,
    uplinks: mpsc::Receiver<LinkPacket>,
    gateways: Vec<KeyedUri>,
    routing_height: u64,
    default_client: router::Service,
    router_broadcast: broadcast::Sender<RouterBroadcast>,
    routers: HashMap<RouterKey, JoinHandle<Result>>,
}

#[derive(PartialEq, Eq, Hash)]
struct RouterKey {
    oui: u32,
    uri: Uri,
}

#[derive(Clone)]
pub enum RouterBroadcast {
    LinkPacket(LinkPacket),
    Routing(Routing),
}

impl Dispatcher {
    // Allow mutable key type for HashMap with Uri in the key
    #[allow(clippy::mutable_key_type)]
    pub fn new(
        shutdown: triggered::Listener,
        downlinks: mpsc::Sender<LinkPacket>,
        uplinks: mpsc::Receiver<LinkPacket>,
        settings: &Settings,
    ) -> Result<Self> {
        let gateways = settings.gateways.clone();
        let router_settings = settings.default_router().clone();
        let default_client = router::Service::new(KeyedUri {
            uri: router_settings.uri,
            public_key: router_settings.public_key,
        })?;
        let (router_broadcast, _) = broadcast::channel(20);
        let routers = HashMap::new();
        Ok(Self {
            shutdown,
            keypair: settings.keypair.clone(),
            region: settings.region,
            uplinks,
            downlinks,
            gateways,
            routing_height: 0,
            default_client,
            router_broadcast,
            routers,
        })
    }

    pub async fn run(&mut self, logger: &Logger) -> Result {
        let logger = logger.new(o!("module" => "dispatcher"));
        info!(logger, "starting");
        info!(logger, "default router";
            "public_key" => self.default_client.uri.public_key.to_string(),
            "uri" => self.default_client.uri.uri.to_string());

        loop {
            let mut gateway = gateway::Service::random_new(&self.gateways)?;
            info!(logger, "selected gateway";
                "public_key" => gateway.uri.public_key.to_string(),
                "uri" => gateway.uri.uri.to_string());
            tokio::select! {
                    _ = self.shutdown.clone() => {
                        info!(logger, "shutting down");
                        return Ok(())
                    },
                    routing_stream = gateway.routing(self.routing_height) => {
                        match routing_stream {
                            Ok(stream) => self.run_with_routing_stream(stream, self.shutdown.clone(), &logger).await?,
                            Err(err) => warn!(logger, "routing error: {:?}", err)
                        }
                        // Check if trigger happened in run_with_routing_stream
                        if self.shutdown.is_triggered() {
                            return Ok(())
                        } else {
                            // Wait a bit before trying another gateway service
                            time::sleep(Duration::from_secs(5)).await;
                        }
                    }
            }
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
                    Ok(Some(response)) => self.handle_routing_update(logger, &response),
                    Ok(None) => {return Ok(())},
                    Err(err) => {
                        info!(logger, "routing error: {:?}", err);
                        return Ok(())
                    }
                },
                uplink = self.uplinks.recv() => match uplink {
                    Some(packet) => {
                        let _ = self.router_broadcast.send(RouterBroadcast::LinkPacket(packet));
                    }
                    None => warn!(logger, "ignoring closed uplinks channel"),
                },
            }
        }
    }

    fn handle_routing_update(&mut self, logger: &Logger, response: &gateway::Response) {
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
        routing_protos
            .iter()
            .for_each(|proto| match Routing::from_proto(logger, proto) {
                Ok(routing) => self.handle_oui_routing_update(logger, routing),
                Err(err) => warn!(logger, "failed to parse routing: {:?}", err),
            });
        self.routing_height = update_height;
        info!(
            logger,
            "updated routing to height {:?}", self.routing_height
        )
    }

    fn handle_oui_routing_update(&mut self, logger: &Logger, routing: Routing) {
        info!(logger, "ROUTING: {:?}", routing);
        routing.uris.iter().for_each(|uri| {
            let key = RouterKey {
                oui: routing.oui,
                uri: uri.uri.clone(),
            };
            if let hash_map::Entry::Vacant(entry) = self.routers.entry(key) {
                match RouterClient::new(
                    self.region,
                    uri.clone(),
                    routing.clone(),
                    self.router_broadcast.subscribe(),
                    self.downlinks.clone(),
                    self.keypair.clone(),
                ) {
                    Ok(mut router) => {
                        let shutdown = self.shutdown.clone();
                        // We stsart the router cope at the root logger to avoid
                        // picking up the dispatched KV pairs
                        let logger = slog_scope::logger();
                        let join_handle =
                            tokio::spawn(async move { router.run(shutdown, &logger).await });
                        entry.insert(join_handle);
                    }
                    Err(err) => {
                        warn!(logger, "faild to construct router: {:?}", err);
                        return;
                    }
                }
            }
        });
        // Remove any routers that are not in the new oui uri list
        self.routers.retain(|key, _| {
            debug!(logger, "removing router";
                "oui" => key.oui,
                "uri" => key.uri.to_string()
            );
            routing.uris.iter().any(|u| u.uri == key.uri)
        });
        // Then broadcast the new routing info to new/existing routers
        let _ = self
            .router_broadcast
            .send(RouterBroadcast::Routing(routing));
    }
}
