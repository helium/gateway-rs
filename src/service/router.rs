use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Error, Keypair, MsgSign, Result,
};

use helium_proto::services::{
    router::{
        envelope_down_v1, envelope_up_v1, EnvelopeDownV1, EnvelopeUpV1, PacketRouterClient,
        PacketRouterPacketDownV1, PacketRouterPacketUpV1, PacketRouterRegisterV1,
    },
    Channel, Endpoint,
};

use http::Uri;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

type PacketClient = PacketRouterClient<Channel>;

type PacketSender = mpsc::Sender<EnvelopeUpV1>;
type PacketReceiver = tonic::Streaming<EnvelopeDownV1>;

#[derive(Debug)]
pub struct RouterService {
    pub uri: Uri,
    packet_router_client: PacketClient,
    conduit: Option<(PacketSender, PacketReceiver)>,
    keypair: Arc<Keypair>,
}

pub const CONDUIT_CAPACITY: usize = 50;

impl RouterService {
    pub fn new(uri: Uri, keypair: Arc<Keypair>) -> Self {
        let packet_channel = Endpoint::from(uri.clone())
            .timeout(RPC_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .connect_lazy();
        Self {
            uri,
            packet_router_client: PacketClient::new(packet_channel),
            conduit: None,
            keypair,
        }
    }

    pub async fn route(&mut self, msg: PacketRouterPacketUpV1) -> Result<()> {
        self.send(msg).await?;
        Ok(())
    }

    pub async fn send(&mut self, msg: PacketRouterPacketUpV1) -> Result {
        if self.conduit.is_none() {
            self.connect().await?;
        }

        let (tx, _) = self.conduit.as_ref().unwrap();
        let msg = EnvelopeUpV1 {
            data: Some(envelope_up_v1::Data::Packet(msg)),
        };
        Ok(tx.send(msg).await?)
    }

    pub async fn mk_conduit(&mut self) -> Result<(PacketSender, PacketReceiver)> {
        let (tx, client_rx) = mpsc::channel(CONDUIT_CAPACITY);
        let rx = self
            .packet_router_client
            .route(ReceiverStream::new(client_rx))
            .await?
            .into_inner();
        Ok((tx, rx))
    }

    pub async fn message(&mut self) -> Result<Option<PacketRouterPacketDownV1>> {
        if self.conduit.is_none() {
            futures::future::pending::<()>().await;
            return Ok(None);
        }

        let (_, rx) = self.conduit.as_mut().unwrap();

        match rx.message().await {
            Ok(Some(msg)) => match msg.data {
                Some(envelope_down_v1::Data::Packet(packet)) => Ok(Some(packet)),
                None => Ok(None),
            },
            Ok(None) => {
                self.disconnect();
                Ok(None)
            }
            Err(err) => {
                self.disconnect();
                Err(err.into())
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.conduit = None;
    }

    pub async fn connect(&mut self) -> Result<()> {
        self.conduit = Some(self.mk_conduit().await?);
        self.register().await?;
        Ok(())
    }

    async fn register(&mut self) -> Result {
        let (tx, _) = self.conduit.as_ref().unwrap();
        let mut register_msg = PacketRouterRegisterV1 {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(Error::from)?
                .as_millis() as u64,
            gateway: self.keypair.public_key().into(),
            signature: vec![],
        };
        register_msg.signature = register_msg.sign(self.keypair.clone()).await?;
        let msg = EnvelopeUpV1 {
            data: Some(envelope_up_v1::Data::Register(register_msg)),
        };
        Ok(tx.send(msg).await?)
    }
}
