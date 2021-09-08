use crate::{
    service::router::{Service as RouterService, StateChannelService},
    CacheSettings, KeyedUri, Keypair, LinkPacket, Region, Result,
};
use slog::{debug, info, o, warn, Logger};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct RouterClient {
    pub(crate) client: RouterService,
    pub(crate) oui: u32,
    pub(crate) region: Region,
    pub(crate) keypair: Arc<Keypair>,
    pub(crate) downlinks: mpsc::Sender<LinkPacket>,
    // store: RouterStore,
    state_channel: StateChannelService,
}

impl RouterClient {
    pub fn new(
        oui: u32,
        region: Region,
        uri: KeyedUri,
        downlinks: mpsc::Sender<LinkPacket>,
        keypair: Arc<Keypair>,
        _settings: CacheSettings,
    ) -> Result<Self> {
        let mut client = RouterService::new(uri)?;
        let state_channel = client.state_channel()?;
        // let store = RouterStore::new(&settings.store.join(uri.public_key.to_string()))?;
        Ok(Self {
            client,
            oui,
            region,
            keypair,
            downlinks,
            // store,
            state_channel,
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
                sc_message = self.state_channel.message() => {
                    info!(logger, "RECEIVED {:?}", sc_message)
                }

            }
        }
    }

    // async fn handle_state_channel_uplink(&mut self, logger: &Logger, uplink: LinkPacket) -> Result {
    //     // let gateway_mac = uplink.gateway_mac;
    //     let message = uplink.to_state_channel_offer(&self.keypair, self.region)?;
    //     self.state_channel.send(message.into()).await
    // }

    async fn handle_uplink(&mut self, logger: &Logger, uplink: LinkPacket) -> Result {
        let gateway_mac = uplink.gateway_mac;
        let message = uplink.to_state_channel_packet(&self.keypair, self.region.clone().into())?;
        let mut client = self.client.clone();
        let logger = logger.clone();
        let downlinks = self.downlinks.clone();
        tokio::spawn(async move {
            match client.route(message.into()).await {
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
        Ok(())
    }
}
