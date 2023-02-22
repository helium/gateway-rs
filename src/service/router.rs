use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    KeyedUri, Result,
};
use helium_proto::{
    services::{self, Channel, Endpoint},
    BlockchainStateChannelMessageV1,
};

type RouterClient = services::router::RouterClient<Channel>;

#[derive(Debug)]
pub struct RouterService {
    pub uri: KeyedUri,
    router_client: RouterClient,
}

impl RouterService {
    pub fn new(keyed_uri: KeyedUri) -> Result<Self> {
        let router_channel = Endpoint::from(keyed_uri.uri.clone())
            .timeout(RPC_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .connect_lazy();
        Ok(Self {
            uri: keyed_uri,
            router_client: RouterClient::new(router_channel),
        })
    }

    pub async fn route(
        &mut self,
        msg: BlockchainStateChannelMessageV1,
    ) -> Result<BlockchainStateChannelMessageV1> {
        Ok(self.router_client.route(msg).await?.into_inner())
    }
}
