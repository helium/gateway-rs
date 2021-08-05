use crate::{service::*, *};
use helium_crypto::Verify;
use helium_proto::{
    services::{self, Channel, Endpoint},
    *,
};
use rand::{rngs::OsRng, seq::SliceRandom};
use std::{sync::Arc, time::Duration};

type ServiceClient = services::gateway::Client<Channel>;

pub struct Streaming {
    streaming: tonic::codec::Streaming<GatewayRespV1>,
    verifier: Arc<PublicKey>,
}

#[derive(Debug, Clone)]
pub struct Response(GatewayRespV1);

use log::debug;

impl Streaming {
    pub async fn message(&mut self) -> Result<Option<Response>> {
        match self.streaming.message().await {
            Ok(Some(response)) => {
                // Create a clone with an empty signature
                let mut v = response.clone();
                v.signature = vec![];
                // Encode the clone
                let mut buf = vec![];
                v.encode(&mut buf)?;
                // And verify against signature in the message
                self.verifier.verify(&buf, &response.signature)?;
                Ok(Some(Response(v)))
            }
            Ok(None) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

impl Response {
    pub fn height(&self) -> u64 {
        self.0.height
    }

    pub fn routings(&self) -> Result<&[Routing]> {
        match &self.0.msg {
            Some(gateway_resp_v1::Msg::RoutingStreamedResp(routings)) => {
                debug!("Received routings");
                Ok(&routings.routings)
            },
            msg => Err(Error::custom(format!(
                "Unexpected gateway message {:?}",
                msg
            ))),
        }
    }
}

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

    pub async fn routing(&mut self, height: u64) -> Result<Streaming> {
        let stream = self.client.routing(GatewayRoutingReqV1 { height }).await?;
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
