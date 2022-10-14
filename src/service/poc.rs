use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Keypair, MsgSign, Result,
};
use helium_proto::services::{
    self,
    poc_lora::{LoraBeaconReportReqV1, LoraWitnessReportReqV1},
    Channel, Endpoint,
};
use http::Uri;
use std::sync::Arc;

type PocLoraClient = services::poc_lora::Client<Channel>;

#[derive(Debug)]
pub struct PocLoraService(PocLoraClient);

impl PocLoraService {
    pub fn new(uri: Uri) -> Self {
        let channel = Endpoint::from(uri)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(RPC_TIMEOUT)
            .connect_lazy();
        let client = services::poc_lora::Client::new(channel);
        Self(client)
    }

    pub async fn submit_beacon(
        &mut self,
        mut req: LoraBeaconReportReqV1,
        keypair: Arc<Keypair>,
    ) -> Result {
        req.pub_key = keypair.public_key().to_vec();
        req.signature = req.sign(keypair).await?;
        let _ = self.0.submit_lora_beacon(req).await?;
        Ok(())
    }

    pub async fn submit_witness(&mut self, req: LoraWitnessReportReqV1) -> Result {
        let _ = self.0.submit_lora_witness(req).await?;
        Ok(())
    }
}
