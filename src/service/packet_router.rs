use crate::{
    impl_sign,
    service::conduit::{ConduitClient, ConduitService},
    DecodeError, Error, Keypair, PublicKey, Result, Sign,
};
use helium_proto::{
    services::{
        router::{
            envelope_down_v1, envelope_up_v1, EnvelopeDownV1, EnvelopeUpV1, PacketRouterClient,
            PacketRouterPacketUpV1, PacketRouterRegisterV1, PacketRouterSessionInitV1,
        },
        Channel,
    },
    Message,
};
use http::Uri;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::async_trait;

// The router service maintains a re-connectable connection to a remote packet
// router. The service will connect when (re)connect or a packet send is
// attempted. It will ensure that the register rpc is called on the constructed
// connection before a packet is sent.
pub struct PacketRouterService(
    ConduitService<EnvelopeUpV1, EnvelopeDownV1, PacketRouterConduitClient>,
);

pub struct PacketRouterConduitClient {}

#[async_trait]
impl ConduitClient<EnvelopeUpV1, EnvelopeDownV1> for PacketRouterConduitClient {
    async fn init(
        &mut self,
        endpoint: Channel,
        tx: mpsc::Sender<EnvelopeUpV1>,
        client_rx: ReceiverStream<EnvelopeUpV1>,
        keypair: Arc<Keypair>,
    ) -> Result<tonic::Streaming<EnvelopeDownV1>> {
        let mut client = PacketRouterClient::<Channel>::new(endpoint);
        let rx = client.route(client_rx).await?.into_inner();
        let mut msg = PacketRouterRegisterV1 {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(Error::from)?
                .as_millis() as u64,
            gateway: keypair.public_key().into(),
            signature: vec![],
            session_capable: true,
        };
        msg.sign(keypair.clone()).await?;
        let msg = EnvelopeUpV1 {
            data: Some(envelope_up_v1::Data::Register(msg)),
        };
        tx.send(msg).await.map_err(|_| Error::channel())?;
        Ok(rx)
    }

    async fn mk_session_init(
        &self,
        nonce: &[u8],
        session_key: &PublicKey,
        keypair: Arc<Keypair>,
    ) -> Result<EnvelopeUpV1> {
        let mut session_init = PacketRouterSessionInitV1 {
            gateway: keypair.public_key().into(),
            session_key: session_key.into(),
            nonce: nonce.to_vec(),
            signature: vec![],
        };
        session_init.sign(keypair).await?;
        let envelope = EnvelopeUpV1 {
            data: Some(envelope_up_v1::Data::SessionInit(session_init)),
        };
        Ok(envelope)
    }
}

impl_sign!(PacketRouterRegisterV1);
impl_sign!(PacketRouterPacketUpV1);
impl_sign!(PacketRouterSessionInitV1);

impl std::ops::Deref for PacketRouterService {
    type Target = ConduitService<EnvelopeUpV1, EnvelopeDownV1, PacketRouterConduitClient>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for PacketRouterService {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl PacketRouterService {
    pub fn new(uri: Uri, keypair: Arc<Keypair>) -> Self {
        let client = PacketRouterConduitClient {};
        Self(ConduitService::new("packet_router", uri, client, keypair))
    }

    pub async fn send_uplink(&mut self, mut msg: PacketRouterPacketUpV1) -> Result {
        self.session_sign(&mut msg).await?;
        let msg = EnvelopeUpV1 {
            data: Some(envelope_up_v1::Data::Packet(msg)),
        };
        self.0.send(msg).await
    }

    pub async fn recv(&mut self) -> Result<envelope_down_v1::Data> {
        self.0.recv().await.and_then(|msg| match msg.data {
            Some(data) => Ok(data),
            None => Err(DecodeError::invalid_envelope()),
        })
    }
}
