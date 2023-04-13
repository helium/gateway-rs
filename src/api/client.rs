use super::{AddGatewayReq, GatewayStakingMode, PubkeyReq, RegionReq, RouterReq};
use crate::{
    error::{DecodeError, Error},
    packet_router::RouterStatus,
    settings::{ListenAddress, StakingMode},
    PublicKey, Region, Result,
};
use helium_proto::{
    services::local::Client, BlockchainTxn, BlockchainTxnAddGatewayV1, Message, Txn,
};
use std::convert::TryFrom;
use tonic::transport::{Channel, Endpoint};

pub struct LocalClient {
    client: Client<Channel>,
}

impl LocalClient {
    pub async fn new(address: &ListenAddress) -> Result<Self> {
        let uri = http::Uri::try_from(address)?;
        let endpoint = Endpoint::from_shared(uri.to_string()).unwrap();
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
        Ok(Region::from_i32(response.into_inner().region)?)
    }

    pub async fn router(&mut self) -> Result<RouterStatus> {
        let response = self.client.router(RouterReq {}).await?;
        response.into_inner().try_into()
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
        let envelope = BlockchainTxn::decode(encoded.as_ref())?;
        match envelope.txn {
            Some(Txn::AddGateway(txn)) => Ok(txn),
            _ => Err(DecodeError::invalid_envelope()),
        }
    }
}
