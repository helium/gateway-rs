use crate::{service::CONNECT_TIMEOUT, Error, KeyedUri, MsgVerify, Result};
use helium_crypto::PublicKey;
use helium_proto::{
    gateway_resp_v1,
    services::{self, Channel, Endpoint},
    BlockchainTxnStateChannelCloseV1, GatewayRespV1, GatewayRoutingReqV1, GatewayScCloseReqV1,
    GatewayScFollowReqV1, GatewayScFollowStreamedRespV1, GatewayScIsActiveReqV1,
    GatewayScIsActiveRespV1, Routing,
};
use rand::{rngs::OsRng, seq::SliceRandom};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

type GatewayClient = services::gateway::Client<Channel>;

pub struct Streaming {
    streaming: tonic::Streaming<GatewayRespV1>,
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
pub struct StateChannelFollowService {
    tx: mpsc::Sender<GatewayScFollowReqV1>,
    rx: tonic::Streaming<GatewayRespV1>,
}

impl StateChannelFollowService {
    pub async fn new(mut client: GatewayClient) -> Result<Self> {
        let (tx, client_rx) = mpsc::channel(3);
        let rx = client
            .follow_sc(ReceiverStream::new(client_rx))
            .await?
            .into_inner();
        Ok(Self { tx, rx })
    }

    pub async fn send(&mut self, id: &[u8], owner: &[u8]) -> Result {
        let msg = GatewayScFollowReqV1 {
            sc_id: id.into(),
            sc_owner: owner.into(),
        };
        Ok(self.tx.send(msg).await?)
    }

    pub async fn message(&mut self) -> Result<Option<GatewayScFollowStreamedRespV1>> {
        use helium_proto::gateway_resp_v1::Msg;
        match self.rx.message().await {
            Ok(Some(GatewayRespV1 {
                msg: Some(Msg::FollowStreamedResp(resp)),
                ..
            })) => Ok(Some(resp)),
            Ok(None) => Ok(None),
            Ok(msg) => Err(Error::custom(format!(
                "unexpected gateway response {:?}",
                msg
            ))),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GatewayService {
    pub uri: KeyedUri,
    client: GatewayClient,
}

impl GatewayService {
    pub fn new(keyed_uri: KeyedUri) -> Result<Self> {
        let channel = Endpoint::from(keyed_uri.uri.clone())
            .timeout(Duration::from_secs(CONNECT_TIMEOUT))
            .connect_lazy()?;
        Ok(Self {
            uri: keyed_uri,
            client: GatewayClient::new(channel),
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

    pub async fn is_active_sc(
        &mut self,
        id: &[u8],
        owner: &[u8],
    ) -> Result<GatewayScIsActiveRespV1> {
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
            Some(gateway_resp_v1::Msg::IsActiveResp(resp)) => {
                let GatewayScIsActiveRespV1 {
                    sc_id, sc_owner, ..
                } = &resp;
                if sc_id == id && sc_owner == owner {
                    Ok(resp)
                } else {
                    Err(Error::custom("mismatched state channel id and owner"))
                }
            }
            Some(other) => Err(Error::custom(format!(
                "invalid is_active response {:?}",
                other
            ))),
            None => Err(Error::custom("empty is_active response")),
        }
    }

    pub async fn follow_sc(&mut self) -> Result<StateChannelFollowService> {
        StateChannelFollowService::new(self.client.clone()).await
    }

    pub async fn close_sc(&mut self, close_txn: BlockchainTxnStateChannelCloseV1) -> Result {
        let _ = self
            .client
            .close_sc(GatewayScCloseReqV1 {
                close_txn: Some(close_txn),
            })
            .await?;
        Ok(())
    }
}
