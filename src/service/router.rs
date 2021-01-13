use crate::*;
use helium_proto::{
    services::{self, Channel, Endpoint},
    BlockchainStateChannelMessageV1,
};
use service::CONNECT_TIMEOUT;
use std::{sync::Arc, time::Duration};

type ServiceClient = services::router::Client<Channel>;

#[derive(Debug, Clone)]
pub struct Service {
    pub uri: http::Uri,
    pub verifier: Option<Arc<PublicKey>>,
    client: ServiceClient,
}

impl Service {
    pub fn new(uri: http::Uri, verifier: Option<PublicKey>) -> Result<Self> {
        let channel = Endpoint::from(uri.clone())
            .timeout(Duration::from_secs(CONNECT_TIMEOUT))
            .connect_lazy()?;
        Ok(Self {
            uri,
            client: ServiceClient::new(channel),
            verifier: verifier.map(Arc::new),
        })
    }

    pub async fn route(
        &mut self,
        msg: BlockchainStateChannelMessageV1,
    ) -> Result<BlockchainStateChannelMessageV1> {
        Ok(self.client.route(msg).await?.into_inner())
    }
}
