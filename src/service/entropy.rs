use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Result,
};
use beacon::Entropy;
use helium_proto::services::{self, poc_entropy::EntropyReqV1, Channel, Endpoint};
use http::Uri;

type EntropyClient = helium_proto::services::poc_entropy::Client<Channel>;

#[derive(Debug)]
pub struct EntropyService(EntropyClient);

impl EntropyService {
    pub fn new(uri: Uri) -> Self {
        let channel = Endpoint::from(uri)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(RPC_TIMEOUT)
            .connect_lazy();
        let client = services::poc_entropy::Client::new(channel);
        Self(client)
    }

    pub async fn get_entropy(&mut self) -> Result<Entropy> {
        let req = EntropyReqV1 {};
        let resp = self.0.entropy(req).await?;
        Ok(resp.into_inner().into())
    }
}
