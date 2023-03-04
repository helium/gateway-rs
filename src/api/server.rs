use super::{
    listen_addr, AddGatewayReq, AddGatewayRes, PubkeyReq, PubkeyRes, RegionReq, RegionRes, SignReq,
    SignRes,
};
use crate::{
    region_watcher, settings::StakingMode, Error, Keypair, PublicKey, Result, Settings,
    TxnEnvelope, TxnFee, TxnFeeConfig,
};
use futures::TryFutureExt;
use helium_crypto::Sign;
use helium_proto::services::local::{Api, Server};
use helium_proto::{BlockchainTxnAddGatewayV1, Message};
use std::{net::SocketAddr, sync::Arc};
use tonic::{self, transport::Server as TransportServer, Request, Response, Status};
use tracing::info;

pub type ApiResult<T> = std::result::Result<Response<T>, Status>;

pub struct LocalServer {
    region_watch: region_watcher::MessageReceiver,
    keypair: Arc<Keypair>,
    onboarding_key: PublicKey,
    listen_port: u16,
}

impl LocalServer {
    pub fn new(region_watch: region_watcher::MessageReceiver, settings: &Settings) -> Result<Self> {
        Ok(Self {
            keypair: settings.keypair.clone(),
            onboarding_key: settings.onboarding_key(),
            listen_port: settings.api,
            region_watch,
        })
    }

    pub async fn run(self, shutdown: &triggered::Listener) -> Result {
        let addr: SocketAddr = listen_addr(self.listen_port).parse().unwrap();
        tracing::Span::current().record("listen", addr.to_string());
        info!(listen = %addr, "starting");
        TransportServer::builder()
            .add_service(Server::new(self))
            .serve_with_shutdown(addr, shutdown.clone())
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

    async fn sign(&self, request: Request<SignReq>) -> ApiResult<SignRes> {
        let data = request.into_inner().data;
        let signature = self
            .keypair
            .sign(&data)
            .map_err(|_err| Status::internal("Failed signing data"))?;
        let reply = SignRes { signature };
        Ok(Response::new(reply))
    }

    async fn add_gateway(&self, request: Request<AddGatewayReq>) -> ApiResult<AddGatewayRes> {
        let request = request.into_inner();
        let _ = PublicKey::from_bytes(&request.owner)
            .map_err(|_err| Status::invalid_argument("Invalid owner address"))?;
        let _ = PublicKey::from_bytes(&request.payer)
            .map_err(|_err| Status::invalid_argument("Invalid payer address"))?;

        let mode = StakingMode::from(request.staking_mode());
        let fee_config = TxnFeeConfig::default();
        let mut txn = BlockchainTxnAddGatewayV1 {
            gateway: self.keypair.public_key().to_vec(),
            owner: request.owner.clone(),
            payer: request.payer,
            fee: 0,
            staking_fee: fee_config.get_staking_fee(&mode),
            owner_signature: vec![],
            gateway_signature: vec![],
            payer_signature: vec![],
        };

        txn.fee = txn
            .txn_fee(&fee_config)
            .map_err(|_err| Status::internal("Failed to get txn fees"))?;

        let signature = self
            .keypair
            .sign(&txn.encode_to_vec())
            .map_err(|_err| Status::internal("Failed signing txn"))?;
        txn.gateway_signature = signature;

        let add_gateway_txn = txn
            .in_envelope_vec()
            .map_err(|_err| Status::internal("Failed to encode txn"))?;
        Ok(Response::new(AddGatewayRes { add_gateway_txn }))
    }
}
