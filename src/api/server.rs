use super::{
    AddGatewayReq, AddGatewayRes, PubkeyReq, PubkeyRes, RegionReq, RegionRes, RouterReq, RouterRes,
};
use crate::{packet_router, region_watcher, Error, Keypair, PublicKey, Result, Settings};
use futures::TryFutureExt;
use helium_crypto::Sign;
use helium_proto::services::local::{Api, Server};
use helium_proto::{BlockchainTxn, BlockchainTxnAddGatewayV1, Message, Txn};
use std::{net::SocketAddr, sync::Arc};
use tonic::{self, transport::Server as TransportServer, Request, Response, Status};
use tracing::info;

pub type ApiResult<T> = std::result::Result<Response<T>, Status>;

pub struct LocalServer {
    region_watch: region_watcher::MessageReceiver,
    packet_router: packet_router::MessageSender,
    keypair: Arc<Keypair>,
    onboarding_key: PublicKey,
    listen_addr: SocketAddr,
}

impl LocalServer {
    pub fn new(
        region_watch: region_watcher::MessageReceiver,
        packet_router: packet_router::MessageSender,
        settings: &Settings,
    ) -> Result<Self> {
        Ok(Self {
            keypair: settings.keypair.clone(),
            onboarding_key: settings.onboarding_key(),
            listen_addr: (&settings.api).try_into()?,
            region_watch,
            packet_router,
        })
    }

    pub async fn run(self, shutdown: &triggered::Listener) -> Result {
        let listen_addr = self.listen_addr;
        tracing::Span::current().record("listen", &listen_addr.to_string());
        info!(listen = %listen_addr, "starting");
        TransportServer::builder()
            .add_service(Server::new(self))
            .serve_with_shutdown(listen_addr, shutdown.clone())
            .map_err(Error::from)
            .await
    }
}

#[tonic::async_trait]
impl Api for LocalServer {
    async fn pubkey(&self, _request: Request<PubkeyReq>) -> ApiResult<PubkeyRes> {
        let reply = PubkeyRes {
            address: self.keypair.public_key().to_vec(),
            onboarding_address: self.onboarding_key.to_vec(),
        };
        Ok(Response::new(reply))
    }

    async fn region(&self, _request: Request<RegionReq>) -> ApiResult<RegionRes> {
        let region_params = self.region_watch.borrow();
        Ok(Response::new(RegionRes {
            region: region_params.region.into(),
        }))
    }

    async fn router(&self, _request: Request<RouterReq>) -> ApiResult<RouterRes> {
        let router_status = self
            .packet_router
            .status()
            .map_err(|_err| Status::internal("Failed to get router status"))
            .await?;
        Ok(Response::new(RouterRes {
            uri: router_status.uri.to_string(),
            connected: router_status.connected,
            session_key: router_status
                .session_key
                .map(|k| k.to_vec())
                .unwrap_or_default(),
        }))
    }

    async fn add_gateway(&self, request: Request<AddGatewayReq>) -> ApiResult<AddGatewayRes> {
        let request = request.into_inner();
        let _ = PublicKey::from_bytes(&request.owner)
            .map_err(|_err| Status::invalid_argument("Invalid owner address"))?;
        let _ = PublicKey::from_bytes(&request.payer)
            .map_err(|_err| Status::invalid_argument("Invalid payer address"))?;

        let mut txn = BlockchainTxnAddGatewayV1 {
            gateway: self.keypair.public_key().to_vec(),
            owner: request.owner.clone(),
            payer: request.payer,
            ..Default::default()
        };

        let signature = self
            .keypair
            .sign(&txn.encode_to_vec())
            .map_err(|_err| Status::internal("Failed signing txn"))?;
        txn.gateway_signature = signature;

        let add_gateway_txn = BlockchainTxn {
            txn: Some(Txn::AddGateway(txn)),
        }
        .encode_to_vec();
        Ok(Response::new(AddGatewayRes { add_gateway_txn }))
    }
}
