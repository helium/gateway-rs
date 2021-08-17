use crate::{service::router, KeyedUri, Keypair, LinkPacket, Region, Result};
use slog::{debug, info, o, warn, Logger};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct RouterClient {
    pub(crate) client: router::Service,
    pub(crate) oui: u32,
    pub(crate) region: Region,
    pub(crate) keypair: Arc<Keypair>,
    pub(crate) downlinks: mpsc::Sender<LinkPacket>,
}

impl RouterClient {
    pub fn new(
        oui: u32,
        region: Region,
        uri: KeyedUri,
        downlinks: mpsc::Sender<LinkPacket>,
        keypair: Arc<Keypair>,
    ) -> Result<Self> {
        let client = router::Service::new(uri)?;
        Ok(Self {
            client,
            oui,
            region,
            keypair,
            downlinks,
        })
    }

    pub async fn run(
        &mut self,
        mut uplinks: mpsc::Receiver<LinkPacket>,
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
                    Some(packet) => match self.handle_uplink(&logger, packet).await {
                        Ok(()) =>  (),
                        Err(err) => warn!(logger, "ignoring failed uplink {:?}", err)
                    },
                    None => warn!(logger, "ignoring closed uplinks channel"),
                },
            }
        }
    }

    async fn handle_uplink(&mut self, logger: &Logger, uplink: LinkPacket) -> Result {
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
}
