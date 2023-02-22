use crate::{
    impl_msg_sign,
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    KeyedUri, Keypair, MsgSign, Region, RegionParams, Result,
};
use helium_proto::services::{self, iot_config::GatewayRegionParamsReqV1, Channel, Endpoint};
use std::sync::Arc;

type ConfigClient = services::iot_config::GatewayClient<Channel>;

impl_msg_sign!(GatewayRegionParamsReqV1, signature);

#[derive(Debug, Clone)]
pub struct ConfigService {
    pub uri: KeyedUri,
    client: ConfigClient,
}

impl ConfigService {
    pub fn new(keyed_uri: &KeyedUri) -> Self {
        let channel = Endpoint::from(keyed_uri.uri.clone())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(RPC_TIMEOUT)
            .connect_lazy();
        Self {
            uri: keyed_uri.clone(),
            client: ConfigClient::new(channel),
        }
    }

    pub async fn region_params(
        &mut self,
        default_region: Region,
        keypair: Arc<Keypair>,
    ) -> Result<RegionParams> {
        let mut req = GatewayRegionParamsReqV1 {
            region: default_region.into(),
            address: keypair.public_key().to_vec(),
            signature: vec![],
        };
        req.signature = req.sign(keypair).await?;

        let resp = self.client.region_params(req).await?.into_inner();
        // TODO: re-enable when config service public prod key is established
        // resp.verify(&self.uri.pubkey)?;
        Ok(RegionParams::try_from(resp)?)
    }
}
