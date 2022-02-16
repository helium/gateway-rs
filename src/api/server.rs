use super::{
    listen_addr, ConfigReq, ConfigRes, ConfigValue, EcdhReq, EcdhRes, HeightReq, HeightRes,
    PubkeyReq, PubkeyRes, RegionReq, RegionRes, SignReq, SignRes,
};
use crate::{router::dispatcher, Error, Keypair, PublicKey, Result, Settings};
use futures::TryFutureExt;
use helium_crypto::Sign;
use helium_proto::services::local::{Api, Server};
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

    async fn ecdh(&self, request: Request<EcdhReq>) -> ApiResult<EcdhRes> {
        let public_key = PublicKey::from_bytes(request.into_inner().address)
            .map_err(|_err| Status::internal("Invalid public key"))?;
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
        let reply = self
            .dispatcher
            .config(&keys)
            .map_err(|_err| Status::internal("Failed to get config"))
            .await?;
        let values = reply.into_iter().map(ConfigValue::from).collect();
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
