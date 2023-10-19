use crate::{
    impl_sign,
    service::conduit::{ConduitClient, ConduitService},
    DecodeError, Keypair, PublicKey, Result, Sign,
};
use helium_proto::{
    services::{
        poc_lora::{
            self, lora_stream_request_v1, lora_stream_response_v1, LoraBeaconReportReqV1,
            LoraStreamRequestV1, LoraStreamResponseV1, LoraWitnessReportReqV1,
        },
        Channel,
    },
    Message as ProtoMessage,
};
use http::Uri;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::async_trait;

// The poc service maintains a re-connectable connection to a remote poc
// ingester. The service will (re)connect when a poc report send is attempted.
// It will ensure that the stream_requests rpc is called on the constructed
// connection before a report is sent.
pub struct PocIotService(
    ConduitService<LoraStreamRequestV1, LoraStreamResponseV1, PocIotConduitClient>,
);

pub struct PocIotConduitClient {}

#[async_trait]
impl ConduitClient<LoraStreamRequestV1, LoraStreamResponseV1> for PocIotConduitClient {
    async fn init(
        &mut self,
        endpoint: Channel,
        _tx: mpsc::Sender<LoraStreamRequestV1>,
        client_rx: ReceiverStream<LoraStreamRequestV1>,
        _keypair: Arc<Keypair>,
    ) -> Result<tonic::Streaming<LoraStreamResponseV1>> {
        let mut client = poc_lora::Client::<Channel>::new(endpoint);
        let rx = client.stream_requests(client_rx).await?.into_inner();
        Ok(rx)
    }

    async fn mk_session_init(
        &self,
        nonce: &[u8],
        session_key: &PublicKey,
        keypair: Arc<Keypair>,
    ) -> Result<LoraStreamRequestV1> {
        let mut session_init = poc_lora::LoraStreamSessionInitV1 {
            pub_key: keypair.public_key().into(),
            session_key: session_key.into(),
            nonce: nonce.to_vec(),
            signature: vec![],
        };
        session_init.sign(keypair).await?;
        let envelope = LoraStreamRequestV1 {
            request: Some(lora_stream_request_v1::Request::SessionInit(session_init)),
        };
        Ok(envelope)
    }
}

impl_sign!(poc_lora::LoraStreamSessionInitV1);
impl_sign!(poc_lora::LoraBeaconReportReqV1);
impl_sign!(poc_lora::LoraWitnessReportReqV1);

impl std::ops::Deref for PocIotService {
    type Target = ConduitService<LoraStreamRequestV1, LoraStreamResponseV1, PocIotConduitClient>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for PocIotService {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl PocIotService {
    pub fn new(module: &'static str, uri: Uri, keypair: Arc<Keypair>) -> Self {
        let client = PocIotConduitClient {};
        Self(ConduitService::new(module, uri, client, keypair))
    }

    pub async fn send(&mut self, msg: lora_stream_request_v1::Request) -> Result {
        let msg = LoraStreamRequestV1 { request: Some(msg) };
        self.0.send(msg).await
    }

    pub async fn recv(&mut self) -> Result<lora_stream_response_v1::Response> {
        self.0.recv().await.and_then(|msg| match msg.response {
            Some(data) => Ok(data),
            None => Err(DecodeError::invalid_envelope()),
        })
    }

    pub async fn submit_beacon(&mut self, mut req: LoraBeaconReportReqV1) -> Result {
        self.0.session_sign(&mut req).await?;
        let msg = lora_stream_request_v1::Request::BeaconReport(req);
        self.send(msg).await
    }

    pub async fn submit_witness(&mut self, mut req: LoraWitnessReportReqV1) -> Result {
        self.0.session_sign(&mut req).await?;
        let msg = lora_stream_request_v1::Request::WitnessReport(req);
        self.send(msg).await
    }
}
