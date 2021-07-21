use super::{RouterBroadcast, Routing};
use crate::{service::router, KeyedUri, Keypair, LinkPacket, Region, Result};
use slog::{debug, info, o, warn, Logger};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

pub struct RouterClient {
    client: router::Service,
    region: Region,
    routing: Routing,
    keypair: Arc<Keypair>,
    uplinks: broadcast::Receiver<RouterBroadcast>,
    downlinks: mpsc::Sender<LinkPacket>,
}

impl RouterClient {
    pub fn new(
        region: Region,
        uri: KeyedUri,
        routing: Routing,
        uplinks: broadcast::Receiver<RouterBroadcast>,
        downlinks: mpsc::Sender<LinkPacket>,
        keypair: Arc<Keypair>,
    ) -> Result<Self> {
        let client = router::Service::new(uri)?;
        Ok(Self {
            uplinks,
            downlinks,
            client,
            routing,
            region,
            keypair,
        })
    }

    pub async fn run(&mut self, shutdown: triggered::Listener, logger: &Logger) -> Result {
        let logger = logger.new(o!(
            "module" => "router",
            "oui" => self.routing.oui,
            "uri" => self.client.uri.uri.to_string()
        ));
        info!(logger, "starting");

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                uplink = self.uplinks.recv() => match uplink {
                    Ok(RouterBroadcast::LinkPacket(packet)) => match self.handle_uplink(&logger, packet).await {
                        Ok(()) =>  (),
                        Err(err) => warn!(logger, "ignoring failed uplink {:?}", err)
                    },
                    Ok(RouterBroadcast::Routing(routing)) => match self.handle_routing_update(&logger, routing) {
                        true => continue,
                        false => info!(logger, "stopping"),
                    },
                    Err(_) => warn!(logger, "ignoring closed uplinks channel"),
                },
            }
        }
    }

    async fn handle_uplink(&mut self, logger: &Logger, uplink: LinkPacket) -> Result {
        if !self.routing.matches_routing_info(&uplink.packet.routing) {
            return Ok(());
        }
        info!(logger, "SENDING UPLINK");
        let gateway_mac = uplink.gateway_mac;
        let message = uplink.to_state_channel_message(&self.keypair, self.region)?;
        match self.client.route(message).await {
            Ok(response) => {
                debug!(logger, "response from router {:?}", response);
                if let Some(downlink) =
                    LinkPacket::from_state_channel_message(response, gateway_mac)
                {
                    match self.downlinks.send(downlink).await {
                        Ok(()) => (),
                        Err(_) => {
                            warn!(logger, "failed to push downlink")
                        }
                    }
                }
            }
            Err(err) => warn!(logger, "ignoring uplink error: {:?}", err),
        }
        Ok(())
    }

    fn handle_routing_update(&mut self, logger: &Logger, routing: Routing) -> bool {
        if self.routing.oui == routing.oui {
            if !routing.uris.contains(&self.client.uri) {
                return false;
            }
            info!(logger, "updating routing");
            self.routing = routing;
        }
        true
    }
}
