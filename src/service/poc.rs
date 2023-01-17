use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Result,
};
use helium_proto::services::{
    self,
    poc_iot::{IotBeaconReportReqV1, IotWitnessReportReqV1},
    Channel, Endpoint,
};
use http::Uri;

type PocIotClient = helium_proto::services::poc_iot::Client<Channel>;

#[derive(Debug)]
pub struct PocIotService(PocIotClient);

impl PocIotService {
    pub fn new(uri: Uri) -> Self {
        let channel = Endpoint::from(uri)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(RPC_TIMEOUT)
            .connect_lazy();
        let client = services::poc_iot::Client::new(channel);
        Self(client)
    }

    pub async fn submit_beacon(&mut self, req: IotBeaconReportReqV1) -> Result {
        _ = self.0.submit_iot_beacon(req).await?;
        Ok(())
    }

    pub async fn submit_witness(&mut self, req: IotWitnessReportReqV1) -> Result {
        _ = self.0.submit_iot_witness(req).await?;
        Ok(())
    }
}
