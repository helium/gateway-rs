use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Error, KeyedUri, Keypair, MsgSign, MsgVerify, PublicKey, Region, Result,
};
use helium_proto::{
    gateway_resp_v1,
    services::{self, Channel, Endpoint},
    BlockchainTxnStateChannelCloseV1, BlockchainVarV1, GatewayConfigReqV1, GatewayConfigRespV1,
    GatewayRegionParamsUpdateReqV1, GatewayRespV1, GatewayRoutingReqV1, GatewayScCloseReqV1,
    GatewayScFollowReqV1, GatewayScFollowStreamedRespV1, GatewayScIsActiveReqV1,
    GatewayScIsActiveRespV1, GatewayValidatorsReqV1, GatewayValidatorsRespV1, GatewayVersionReqV1,
    GatewayVersionRespV1, Routing,
};
use rand::{rngs::OsRng, seq::SliceRandom};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream};

type GatewayClient = services::gateway::Client<Channel>;
pub use crate::service::version::GatewayVersion;

#[derive(Debug)]
pub struct Streaming {
    streaming: tonic::Streaming<GatewayRespV1>,
    verifier: Arc<PublicKey>,
}

impl Stream for Streaming {
    type Item = Result<GatewayRespV1>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.streaming)
            .poll_next(cx)
            .map_err(Error::from)
            .map(|msg| match msg {
                Some(Ok(response)) => Some(response.verify(&self.verifier).map(|_| response)),
                Some(Err(err)) => Some(Err(err)),
                None => None,
            })
    }
}

pub(crate) trait Response {
    fn height(&self) -> u64;
    fn routings(&self) -> Result<&[Routing]>;
    fn region(&self) -> Result<Region>;
    fn state_channel_response(&self) -> Result<&GatewayScFollowStreamedRespV1>;
}

impl Response for GatewayRespV1 {
    fn height(&self) -> u64 {
        self.height
    }

    fn routings(&self) -> Result<&[Routing]> {
        match &self.msg {
            Some(gateway_resp_v1::Msg::RoutingStreamedResp(routings)) => Ok(&routings.routings),
            msg => Err(Error::custom(
                format!("Unexpected gateway message {msg:?}",),
            )),
        }
    }

    fn region(&self) -> Result<Region> {
        match &self.msg {
            Some(gateway_resp_v1::Msg::RegionParamsStreamedResp(params)) => {
                Region::from_i32(params.region)
            }
            msg => Err(Error::custom(
                format!("Unexpected gateway message {msg:?}",),
            )),
        }
    }

    fn state_channel_response(&self) -> Result<&GatewayScFollowStreamedRespV1> {
        match &self.msg {
            Some(gateway_resp_v1::Msg::FollowStreamedResp(res)) => Ok(res),
            msg => Err(Error::custom(
                format!("Unexpected gateway message {msg:?}",),
            )),
        }
    }
}

#[derive(Debug)]
pub struct StateChannelFollowService {
    tx: Option<mpsc::Sender<GatewayScFollowReqV1>>,
    rx: Option<Streaming>,
}

impl StateChannelFollowService {
    pub async fn new(gateway: &mut GatewayService) -> Result<Self> {
        let mut result = Self { tx: None, rx: None };
        result.set_gateway(Some(gateway)).await?;
        Ok(result)
    }

    pub async fn send(&mut self, id: &[u8], owner: &[u8]) -> Result {
        match self.tx.as_mut() {
            Some(tx) => {
                let msg = GatewayScFollowReqV1 {
                    sc_id: id.into(),
                    sc_owner: owner.into(),
                };
                Ok(tx.send(msg).await?)
            }
            None => Err(Error::no_service()),
        }
    }

    pub async fn set_gateway(&mut self, gateway: Option<&mut GatewayService>) -> Result {
        let (tx, rx) = match gateway {
            Some(gateway) => {
                let (tx, client_rx) = mpsc::channel(3);
                let streaming = gateway
                    .client
                    .follow_sc(ReceiverStream::new(client_rx))
                    .await?
                    .into_inner();
                let rx = Streaming {
                    streaming,
                    verifier: gateway.uri.pubkey.clone(),
                };
                (Some(tx), Some(rx))
            }
            None => (None, None),
        };
        self.tx = tx;
        self.rx = rx;
        Ok(())
    }
}

impl Stream for StateChannelFollowService {
    type Item = Result<GatewayRespV1>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(rx) = self.rx.as_mut() {
            Pin::new(rx).poll_next(cx)
        } else {
            Poll::Pending
        }
    }
}

#[derive(Debug, Clone)]
pub struct GatewayService {
    pub uri: KeyedUri,
    client: GatewayClient,
}

impl GatewayService {
    pub fn new(keyed_uri: &KeyedUri) -> Result<Self> {
        let channel = Endpoint::from(keyed_uri.uri.clone())
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
            .timeout(Duration::from_secs(RPC_TIMEOUT))
            .connect_lazy();
        Ok(Self {
            uri: keyed_uri.clone(),
            client: GatewayClient::new(channel),
        })
    }

    pub fn select_seed(seed_uris: &[KeyedUri]) -> Result<Self> {
        seed_uris
            .choose(&mut OsRng)
            .ok_or_else(|| Error::custom("empty uri list"))
            .and_then(Self::new)
    }

    pub async fn random_new(
        &mut self,
        fetch_count: u8,
        cancel: triggered::Listener,
    ) -> Result<Option<Self>> {
        tokio::select! {
            gateways = self.validators(fetch_count.into()) => match gateways {
                Ok(gateways) => gateways
                    .choose(&mut OsRng)
                    .ok_or_else(|| Error::custom("empty gateway list"))
                    .and_then(Self::new)
                    .map(Some),
                Err(err) => Err(err)
            },
            _ = cancel.clone() => Ok(None)
        }
    }

    pub async fn routing(&mut self, height: u64) -> Result<Streaming> {
        let stream = self.client.routing(GatewayRoutingReqV1 { height }).await?;
        Ok(Streaming {
            streaming: stream.into_inner(),
            verifier: self.uri.pubkey.clone(),
        })
    }

    pub async fn region_params(&mut self, keypair: Arc<Keypair>) -> Result<Streaming> {
        let mut req = GatewayRegionParamsUpdateReqV1 {
            address: keypair.public_key().to_vec(),
            signature: vec![],
        };
        req.signature = req.sign(keypair).await?;

        let stream = self.client.region_params_update(req).await?;
        Ok(Streaming {
            streaming: stream.into_inner(),
            verifier: self.uri.pubkey.clone(),
        })
    }

    pub async fn is_active_sc(
        &mut self,
        id: &[u8],
        owner: &[u8],
    ) -> Result<GatewayScIsActiveRespV1> {
        let resp = self
            .client
            .is_active_sc(GatewayScIsActiveReqV1 {
                sc_owner: owner.into(),
                sc_id: id.into(),
            })
            .await?
            .into_inner();
        resp.verify(&self.uri.pubkey)?;
        match resp.msg {
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
                "invalid is_active response {other:?}",
            ))),
            None => Err(Error::custom("empty is_active response")),
        }
    }

    pub async fn follow_sc(&mut self) -> Result<StateChannelFollowService> {
        StateChannelFollowService::new(self).await
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

    async fn get_config(&mut self, keys: Vec<String>) -> Result<GatewayRespV1> {
        let resp = self
            .client
            .config(GatewayConfigReqV1 { keys })
            .await?
            .into_inner();
        resp.verify(&self.uri.pubkey)?;
        Ok(resp)
    }

    pub async fn config(&mut self, keys: Vec<String>) -> Result<Vec<BlockchainVarV1>> {
        match self.get_config(keys).await?.msg {
            Some(gateway_resp_v1::Msg::ConfigResp(GatewayConfigRespV1 { result })) => Ok(result),
            Some(other) => Err(Error::custom(format!("invalid config response {other:?}"))),
            None => Err(Error::custom("empty config response")),
        }
    }

    pub async fn height(&mut self) -> Result<(u64, u64)> {
        let resp = self.get_config(vec![]).await?;
        Ok((resp.height, resp.block_age))
    }

    pub async fn validators(&mut self, quantity: u32) -> Result<Vec<KeyedUri>> {
        let resp = self
            .client
            .validators(GatewayValidatorsReqV1 { quantity })
            .await?
            .into_inner();
        resp.verify(&self.uri.pubkey)?;
        match resp.msg {
            Some(gateway_resp_v1::Msg::ValidatorsResp(GatewayValidatorsRespV1 { result })) => {
                result.into_iter().map(KeyedUri::try_from).collect()
            }
            Some(other) => Err(Error::custom(format!(
                "invalid validator response {other:?}"
            ))),
            None => Err(Error::custom("empty validator response")),
        }
    }

    pub async fn version(&mut self) -> Result<Option<u64>> {
        let resp = self
            .client
            .version(GatewayVersionReqV1 {})
            .await?
            .into_inner();
        resp.verify(&self.uri.pubkey)?;
        match resp.msg {
            Some(gateway_resp_v1::Msg::Version(GatewayVersionRespV1 { version })) => {
                Ok(Some(version))
            }
            Some(other) => Err(Error::custom(format!(
                "invalid validator response {other:?}"
            ))),
            None => Ok(None),
        }
    }
}
