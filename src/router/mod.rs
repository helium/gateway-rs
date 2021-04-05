use crate::*;
use helium_proto::RoutingInformation;
use link_packet::LinkPacket;
use service::{
    gateway::{Response as GatewayResponse, Service as GatewayService, Streaming},
    router::Service as RouterService,
};
use slog::{debug, info, o, warn, Logger};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    time,
};

pub mod filter;
pub mod routing;

pub use helium_proto::Region;
pub use routing::Routing;

pub struct Router {
    downlinks: Sender<LinkPacket>,
    uplinks: Receiver<LinkPacket>,
    region: Region,
    keypair: Arc<Keypair>,
    gateways: Vec<KeyedUri>,
    routing_height: u64,
    clients: HashMap<u32, Routing>,
    default_client: RouterService,
}

impl Router {
    pub fn new(
        downlinks: Sender<LinkPacket>,
        uplinks: Receiver<LinkPacket>,
        settings: &Settings,
    ) -> Result<Self> {
        let gateways = settings.gateways.clone();
        let router_settings = settings.router.clone();
        let default_client =
            RouterService::new(router_settings.uri, Some(router_settings.public_key))?;
        Ok(Self {
            keypair: settings.keypair.clone(),
            region: settings.region,
            uplinks,
            downlinks,
            gateways,
            routing_height: 0,
            clients: HashMap::new(),
            default_client,
        })
    }

    pub async fn run(&mut self, shutdown: triggered::Listener, logger: &Logger) -> Result {
        let logger = logger.new(o!("module" => "router"));
        info!(logger, "starting");
        loop {
            let mut gateway = GatewayService::random_new(&self.gateways)?;
            info!(logger, "selected gateway";
                "public_key" => gateway.verifier.to_string(),
                "uri" => gateway.uri.to_string());
            tokio::select! {
                    _ = shutdown.clone() => {
                        info!(logger, "shutting down");
                        return Ok(())
                    },
                    routing_stream = gateway.routing(self.routing_height) => {
                        match routing_stream {
                            Ok(stream) => self.run_with_routing_stream(stream, shutdown.clone(), &logger).await?,
                            Err(err) => {
                                warn!(logger, "routing error: {:?}", err);
                                time::sleep(Duration::from_secs(5)).await;
                            }
                        }
                    }
            }
        }
    }

    async fn run_with_routing_stream(
        &mut self,
        mut routing_stream: Streaming,
        shutdown: triggered::Listener,
        logger: &Logger,
    ) -> Result {
        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                routing = routing_stream.message() => match routing {
                    Ok(Some(response)) => self.handle_routing_update(&logger, &response),
                    Ok(None) => {return Ok(())},
                    Err(err) => {
                        info!(logger, "routing error: {:?}", err);
                        return Ok(())
                    }
                },
                uplink = self.uplinks.recv() => match uplink {
                    Some(packet) => match self.handle_uplink(&logger, packet).await {
                        Ok(()) =>  (),
                        Err(err) => warn!(logger, "ignoring failed uplink {:?}", err)
                    },
                    None => warn!(logger, "ignoring closed downlinks channel"),
                },
            }
        }
    }

    fn handle_routing_update(&mut self, logger: &Logger, response: &GatewayResponse) {
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
        let routings = match response.routings() {
            Ok(v) => v,
            Err(err) => {
                warn!(logger, "error decoding routing {:?}", err);
                return;
            }
        };
        for routing in routings {
            match routing::Routing::from_proto(routing) {
                Ok(client) => {
                    self.clients.insert(routing.oui, client);
                }
                Err(err) => warn!(logger, "failed to construct router client: {:?}", err),
            }
        }
        self.routing_height = update_height;
        info!(
            logger,
            "updated routing to height {:?}", self.routing_height
        )
    }

    async fn handle_uplink(&mut self, logger: &Logger, uplink: LinkPacket) -> Result {
        if uplink.packet.routing.is_none() {
            info!(logger, "ignoring, no routing data");
            return Ok(());
        };
        let gateway_mac = uplink.gateway_mac;
        let message = uplink.to_state_channel_message(&self.keypair, self.region)?;
        for mut client in self.router_clients_for_uplink(&uplink) {
            let downlinks = self.downlinks.clone();
            let message = message.clone();
            let logger = logger.clone();
            info!(logger, "routing packet to: {}", client.uri);
            tokio::spawn(async move {
                match client.route(message).await {
                    Ok(response) => {
                        debug!(logger, "response from router {:?}", response);
                        if let Some(downlink) =
                            LinkPacket::from_state_channel_message(response, gateway_mac)
                        {
                            match downlinks.send(downlink).await {
                                Ok(()) => (),
                                Err(_) => {
                                    warn!(logger, "failed to push downlink")
                                }
                            }
                        }
                    }
                    Err(err) => warn!(logger, "ignoring uplink error: {:?}", err),
                }
            });
        }
        Ok(())
    }

    fn router_clients_for_uplink(&self, uplink: &LinkPacket) -> Vec<RouterService> {
        match &uplink.packet.routing {
            Some(RoutingInformation {
                data: Some(routing_data),
            }) => {
                let found: Vec<RouterService> = self
                    .clients
                    .values()
                    .filter(|&routing| routing.matches_routing_data(&routing_data))
                    .flat_map(|routing| routing.clients.clone())
                    .collect();
                if found.is_empty() {
                    vec![self.default_client.clone()]
                } else {
                    found
                }
            }
            _ => vec![],
        }
    }
}
