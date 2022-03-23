use super::{
    connect_uri, AddGatewayReq, ConfigReq, ConfigValue, GatewayStakingMode, HeightReq, HeightRes,
    PubkeyReq, RegionReq, SignReq,
};
use crate::{error::Error, settings::StakingMode, PublicKey, Region, Result, TxnEnvelope};
use helium_proto::{services::local::Client, BlockchainTxnAddGatewayV1};
use std::convert::TryFrom;
use tonic::transport::{Channel, Endpoint};

pub struct LocalClient {
    client: Client<Channel>,
}

impl LocalClient {
    pub async fn new(port: u16) -> Result<Self> {
        let uri = connect_uri(port);
        let endpoint = Endpoint::from_shared(uri).unwrap();
        let client = Client::connect(endpoint)
            .await
            .map_err(Error::local_client_connect)?;
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

    pub async fn config<T>(&mut self, keys: &[T]) -> Result<Vec<ConfigValue>>
    where
        T: ToString,
    {
        let keys = keys.iter().map(|s| s.to_string()).collect();
        let response = self.client.config(ConfigReq { keys }).await?.into_inner();
        Ok(response.values)
    }

    pub async fn height(&mut self) -> Result<HeightRes> {
        let response = self.client.height(HeightReq {}).await?.into_inner();
        Ok(response)
    }

    pub async fn region(&mut self) -> Result<Region> {
        let response = self.client.region(RegionReq {}).await?;
        Region::from_i32(response.into_inner().region)
    }

    pub async fn add_gateway(
        &mut self,
        owner: &PublicKey,
        payer: &PublicKey,
        mode: &StakingMode,
    ) -> Result<BlockchainTxnAddGatewayV1> {
        let response = self
            .client
            .add_gateway(AddGatewayReq {
                owner: owner.to_vec(),
                payer: payer.to_vec(),
                staking_mode: GatewayStakingMode::from(mode).into(),
            })
            .await?;
        let encoded = response.into_inner().add_gateway_txn;
        let txn = BlockchainTxnAddGatewayV1::from_envelope_vec(&encoded)?;
        Ok(txn)
    }
}
