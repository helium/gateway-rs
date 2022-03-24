use super::{
    listen_addr, AddGatewayReq, AddGatewayRes, ConfigReq, ConfigRes, ConfigValue, EcdhReq, EcdhRes,
    HeightReq, HeightRes, PubkeyReq, PubkeyRes, RegionReq, RegionRes, SignReq, SignRes,
};
use crate::{
    router::dispatcher, settings::StakingMode, Error, Keypair, PublicKey, Result, Settings,
    TxnEnvelope, TxnFee, TxnFeeConfig, CONFIG_FEE_KEYS,
};
use futures::TryFutureExt;
use helium_crypto::Sign;
use helium_proto::services::local::{Api, Server};
use helium_proto::{BlockchainTxnAddGatewayV1, Message};
use slog::{info, o, Logger};
use std::sync::Arc;
use tonic::{self, transport::Server as TransportServer, Request, Response, Status};

pub type ApiResult<T> = std::result::Result<Response<T>, Status>;

pub struct LocalServer {
    dispatcher: dispatcher::MessageSender,
    keypair: Arc<Keypair>,
    listen_port: u16,
}

impl LocalServer {
    pub fn new(dispatcher: dispatcher::MessageSender, settings: &Settings) -> Self {
        Self {
            keypair: settings.keypair.clone(),
            listen_port: settings.api,
            dispatcher,
        }
    }

    pub async fn run(self, shutdown: triggered::Listener, logger: &Logger) -> Result {
        let addr = listen_addr(self.listen_port).parse().unwrap();
        let logger = logger.new(o!("module" => "api", "listen" => addr));
        info!(logger, "starting");
        TransportServer::builder()
            .add_service(Server::new(self))
            .serve_with_shutdown(addr, shutdown)
            .map_err(Error::from)
            .await
    }

    async fn _get_config<T>(&self, keys: &[T]) -> std::result::Result<Vec<ConfigValue>, Status>
    where
        T: ToString,
    {
        let keys = keys.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        let reply = self
            .dispatcher
            .config(&keys)
            .map_err(|_err| Status::internal("Failed to get config"))
            .await?;
        let values = reply.into_iter().map(ConfigValue::from).collect();
        Ok(values)
    }
}

#[tonic::async_trait]
impl Api for LocalServer {
    async fn pubkey(&self, _request: Request<PubkeyReq>) -> ApiResult<PubkeyRes> {
        let reply = PubkeyRes {
            address: self.keypair.public_key().to_vec(),
        };
        Ok(Response::new(reply))
    }

    async fn region(&self, _request: Request<RegionReq>) -> ApiResult<RegionRes> {
        let region = self
            .dispatcher
            .region()
            .map_err(|_err| Status::internal("Failed to get region"))
            .await?;
        Ok(Response::new(RegionRes {
            region: region.into(),
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
        let config_values = self._get_config(CONFIG_FEE_KEYS).await?;
        let fee_config = TxnFeeConfig::try_from(config_values)
            .map_err(|_err| Status::internal("Failed to parse txn fees"))?;
        let mut txn = BlockchainTxnAddGatewayV1 {
            gateway: self.keypair.public_key().to_vec(),
            owner: request.owner.clone(),
            payer: request.payer.clone(),
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

    async fn ecdh(&self, request: Request<EcdhReq>) -> ApiResult<EcdhRes> {
        let public_key = PublicKey::from_bytes(request.into_inner().address)
            .map_err(|_err| Status::invalid_argument("Invalid public key"))?;
        let secret = self
            .keypair
            .ecdh(&public_key)
            .map_err(|_err| Status::internal("Failed ecdh"))?;
        let reply = EcdhRes {
            secret: secret.as_bytes().to_vec(),
        };
        Ok(Response::new(reply))
    }

    async fn config(&self, request: Request<ConfigReq>) -> ApiResult<ConfigRes> {
        let keys = request.into_inner().keys;
        let values = self._get_config(&keys).await?;
        Ok(Response::new(ConfigRes { values }))
    }

    async fn height(&self, _request: Request<HeightReq>) -> ApiResult<HeightRes> {
        let reply = self
            .dispatcher
            .height()
            .map_err(|_err| Status::internal("Failed to get config"))
            .await?;
        Ok(Response::new(HeightRes {
            height: reply.height,
            block_age: reply.block_age,
            gateway: Some(reply.gateway.into()),
        }))
    }
}
