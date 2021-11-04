use super::service::{Client, PubkeyReq, SignReq, CONNECT_URI};
use crate::{PublicKey, Result};
use std::convert::TryFrom;
use tonic::transport::{Channel, Endpoint};

pub struct GatewayClient {
    client: Client<Channel>,
}

impl GatewayClient {
    pub async fn new() -> Result<Self> {
        let addr = Endpoint::from_static(CONNECT_URI);
        let client = Client::connect(addr).await?;
        Ok(Self { client })
    }

    pub async fn pubkey(&mut self) -> Result<PublicKey> {
        let response = self.client.pubkey(PubkeyReq {}).await?;
        let public_key = PublicKey::try_from(response.into_inner().address)?;
        Ok(public_key)
    }

    pub async fn sign(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let response = self.client.sign(SignReq { data: data.into() }).await?;
        let signature = response.into_inner().signature;
        Ok(signature)
    }
}
