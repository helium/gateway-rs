use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Result,
};
use helium_proto::services::{
    poc_lora::{Client, LoraBeaconReportReqV1, LoraWitnessReportReqV1},
    Channel, Endpoint,
};
use http::Uri;

type PocLoraClient = Client<Channel>;

#[derive(Debug)]
pub struct PocLoraService {
    client: PocLoraClient,
}

impl PocLoraService {
    // TODO: Use keyed URI
    pub fn new(uri: Uri) -> Self {
        let channel = Endpoint::from(uri)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(RPC_TIMEOUT)
            .connect_lazy();
        Self {
            client: PocLoraClient::new(channel),
        }
    }

    pub async fn submit_beacon(&mut self, report: LoraBeaconReportReqV1) -> Result<String> {
        let resp = self.client.submit_lora_beacon(report).await?;
        // TODO: verify with pubkey
        Ok(resp.into_inner().id)
    }

    pub async fn submit_witness(&mut self, report: LoraWitnessReportReqV1) -> Result<String> {
        let resp = self.client.submit_lora_witness(report).await?;
        // TODO: verify with pubkey
        Ok(resp.into_inner().id)
    }
}
