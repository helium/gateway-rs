use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Result,
};
use helium_proto::services::{
    poc_lora::{Client, LoraBeaconReportReqV1, LoraWitnessReportReqV1},
    Channel, Endpoint,
};
use http::Uri;
use tonic::{
    metadata::{Ascii, MetadataValue},
    service::{interceptor::InterceptedService, Interceptor},
};

type PocLoraClient = Client<InterceptedService<Channel, PocLorReqInterceptor>>;

#[derive(Debug)]
pub struct PocLoraService(PocLoraClient);

impl PocLoraService {
    pub fn new(uri: Uri) -> Self {
        let channel = Endpoint::from(uri)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(RPC_TIMEOUT)
            .connect_lazy();
        let metadata_val = std::env::var("POC_LORA_AUTH_TOKEN")
            .ok()
            .map(|token| MetadataValue::try_from(format!("Bearer {}", token)).unwrap());
        let client: PocLoraClient =
            Client::with_interceptor(channel, PocLorReqInterceptor(metadata_val));
        Self(client)
    }

    pub async fn submit_beacon(&mut self, report: LoraBeaconReportReqV1) -> Result<String> {
        let resp = self.0.submit_lora_beacon(report).await?;
        Ok(resp.into_inner().id)
    }

    pub async fn submit_witness(&mut self, report: LoraWitnessReportReqV1) -> Result<String> {
        let resp = self.0.submit_lora_witness(report).await?;
        Ok(resp.into_inner().id)
    }
}

// We need to intercept requests in order to add the auth-token.
struct PocLorReqInterceptor(Option<MetadataValue<Ascii>>);

impl Interceptor for PocLorReqInterceptor {
    fn call(
        &mut self,
        mut request: tonic::Request<()>,
    ) -> std::result::Result<tonic::Request<()>, tonic::Status> {
        if let Some(metadata_val) = &self.0 {
            request
                .metadata_mut()
                .insert("authorization", metadata_val.clone());
        };
        Ok(request)
    }
}
