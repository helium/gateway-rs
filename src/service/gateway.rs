use crate::{service::CONNECT_TIMEOUT, Error, KeyedUri, MsgVerify, Result};
use helium_crypto::PublicKey;
use helium_proto::{
    gateway_resp_v1,
    services::{self, Channel, Endpoint},
    GatewayRespV1, GatewayRoutingReqV1, GatewayScIsActiveReqV1, GatewayScIsActiveRespV1, Routing,
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

impl Streaming {
    pub async fn message(&mut self) -> Result<Option<Response>> {
        match self.streaming.message().await {
            Ok(Some(response)) => {
                response.verify(&self.verifier)?;
                Ok(Some(Response(response)))
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
            Some(gateway_resp_v1::Msg::RoutingStreamedResp(routings)) => Ok(&routings.routings),
            msg => Err(Error::custom(format!(
                "Unexpected gateway message {:?}",
                msg
            ))),
        }
    }
}

#[derive(Debug)]
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

    pub fn random_new(uris: &[KeyedUri]) -> Result<Self> {
        let uri = uris
            .choose(&mut OsRng)
            .ok_or_else(|| Error::custom("empty uri list"))?;
        Self::new(uri.to_owned())
    }

    pub async fn routing(&mut self, height: u64) -> Result<Streaming> {
        let stream = self.client.routing(GatewayRoutingReqV1 { height }).await?;
        Ok(Streaming {
            streaming: stream.into_inner(),
            verifier: self.uri.public_key.clone(),
        })
    }

    pub async fn is_active(&mut self, id: &[u8], owner: &[u8]) -> Result<bool> {
        match self
            .client
            .is_active_sc(GatewayScIsActiveReqV1 {
                sc_owner: owner.into(),
                sc_id: id.into(),
            })
            .await?
            .into_inner()
            .msg
        {
            Some(gateway_resp_v1::Msg::IsActiveResp(GatewayScIsActiveRespV1 {
                sc_id,
                sc_owner,
                active,
            })) => {
                if sc_id == id && sc_owner == owner {
                    Ok(active)
                } else {
                    Err(Error::custom("mismatched state channel id and owner"))
                }
            }
            _ => Ok(false),
        }
    }
}
