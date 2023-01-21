use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Result,
};
use helium_proto::services::{
    self,
    poc_lora::{LoraBeaconReportReqV1, LoraWitnessReportReqV1},
    Channel, Endpoint,
};
use http::Uri;

type PocIotClient = helium_proto::services::poc_lora::Client<Channel>;

#[derive(Debug)]
pub struct PocIotService(PocIotClient);

impl PocIotService {
    pub fn new(uri: Uri) -> Self {
        let channel = Endpoint::from(uri)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(RPC_TIMEOUT)
            .connect_lazy();
        let client = services::poc_lora::Client::new(channel);
        Self(client)
    }

    pub async fn submit_beacon(&mut self, req: LoraBeaconReportReqV1) -> Result {
        _ = self.0.submit_lora_beacon(req).await?;
        Ok(())
    }

    pub async fn submit_witness(&mut self, req: LoraWitnessReportReqV1) -> Result {
        _ = self.0.submit_lora_witness(req).await?;
        Ok(())
    }
}
