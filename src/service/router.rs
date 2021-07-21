use crate::*;
use helium_proto::{
    services::{self, Channel, Endpoint},
    BlockchainStateChannelMessageV1,
};
use service::CONNECT_TIMEOUT;
use std::time::Duration;

type ServiceClient = services::router::Client<Channel>;

#[derive(Debug, Clone)]
pub struct Service {
    pub uri: KeyedUri,
    client: ServiceClient,
}

impl Service {
    pub fn new(keyed_uri: KeyedUri) -> Result<Self> {
        let channel = Endpoint::from(keyed_uri.uri.clone())
            .timeout(Duration::from_secs(CONNECT_TIMEOUT))
            .connect_lazy()?;
        Ok(Self {
            uri: keyed_uri,
            client: ServiceClient::new(channel),
        })
    }

    pub async fn route(
        &mut self,
        msg: BlockchainStateChannelMessageV1,
    ) -> Result<BlockchainStateChannelMessageV1> {
        Ok(self.client.route(msg).await?.into_inner())
    }
}
