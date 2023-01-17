use super::{connect_uri, AddGatewayReq, GatewayStakingMode, PubkeyReq, RegionReq};
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

    pub async fn pubkey(&mut self) -> Result<(PublicKey, PublicKey)> {
        let response = self.client.pubkey(PubkeyReq {}).await?.into_inner();

        let public_key = PublicKey::try_from(response.address)?;
        let onboarding_key = PublicKey::try_from(response.onboarding_address)?;
        Ok((public_key, onboarding_key))
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
