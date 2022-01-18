use super::{connect_uri, ConfigReq, ConfigValue, HeightReq, HeightRes, PubkeyReq, SignReq};
use crate::{PublicKey, Result};
use helium_proto::services::local::Client;
use std::convert::TryFrom;
use tonic::transport::{Channel, Endpoint};

pub struct LocalClient {
    client: Client<Channel>,
}

impl LocalClient {
    pub async fn new(port: u16) -> Result<Self> {
        let uri = connect_uri(port);
        let endpoint = Endpoint::from_shared(uri).unwrap();
        let client = Client::connect(endpoint).await?;
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

    pub async fn config(&mut self, keys: &[&str]) -> Result<Vec<ConfigValue>> {
        let keys = keys.iter().map(|s| s.to_string()).collect();
        let response = self.client.config(ConfigReq { keys }).await?.into_inner();
        Ok(response.values)
    }

    pub async fn height(&mut self) -> Result<HeightRes> {
        let response = self.client.height(HeightReq {}).await?.into_inner();
        Ok(response)
    }
}
