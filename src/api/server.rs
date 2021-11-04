use super::service::{Api, PubkeyReq, PubkeyRes, Server, SignReq, SignRes, LISTEN_ADDR};
use crate::{Error, Keypair, Result, Settings};
use futures::TryFutureExt;
use helium_crypto::Sign;
use slog::{info, o, Logger};
use std::sync::Arc;
use tonic::{self, transport::Server as TransportServer, Request, Response, Status};

pub type ApiResult<T> = std::result::Result<Response<T>, Status>;

pub struct GatewayServer {
    keypair: Arc<Keypair>,
}

impl GatewayServer {
    pub fn new(settings: &Settings) -> Self {
        Self {
            keypair: settings.keypair.clone(),
        }
    }

    pub async fn run(self, shutdown: triggered::Listener, logger: &Logger) -> Result {
        let addr = LISTEN_ADDR.parse().unwrap();
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
impl Api for GatewayServer {
    async fn pubkey(&self, _request: Request<PubkeyReq>) -> ApiResult<PubkeyRes> {
        let reply = PubkeyRes {
            address: self.keypair.public_key().to_vec(),
        };
        Ok(Response::new(reply))
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
}
