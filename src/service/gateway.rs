use crate::*;
use helium_proto::{
    services::{self, Channel, Endpoint},
    RoutingRequest, RoutingResponse,
};
use rand::{rngs::OsRng, seq::SliceRandom};
use service::{SignatureAccess, Streaming, CONNECT_TIMEOUT};
use std::{sync::Arc, time::Duration};

type ServiceClient = services::gateway::Client<Channel>;

#[derive(Debug, Clone)]
pub struct Service {
    pub uri: http::Uri,
    pub verifier: Arc<PublicKey>,
    client: ServiceClient,
}

impl Service {
    pub fn new(keyed_uri: KeyedUri) -> Result<Self> {
        let channel = Endpoint::from(keyed_uri.uri.clone())
            .timeout(Duration::from_secs(CONNECT_TIMEOUT))
            .connect_lazy()?;
        Ok(Self {
            uri: keyed_uri.uri,
            client: ServiceClient::new(channel),
            verifier: Arc::new(keyed_uri.public_key),
        })
    }

    pub async fn routing(&mut self, height: u64) -> Result<Streaming<RoutingResponse>> {
        let stream = self.client.routing(RoutingRequest { height }).await?;
        Ok(Streaming {
            streaming: stream.into_inner(),
            verifier: self.verifier.clone(),
        })
    }

    pub fn random_new(uris: &[KeyedUri]) -> Result<Self> {
        let uri = uris
            .choose(&mut OsRng)
            .ok_or_else(|| Error::custom("empty uri list"))?;
        Self::new(uri.clone())
    }
}

impl SignatureAccess for RoutingResponse {
    fn set_signature(&mut self, signature: Vec<u8>) {
        self.signature = signature;
    }
    fn get_signature(&self) -> &Vec<u8> {
        &self.signature
    }
}
