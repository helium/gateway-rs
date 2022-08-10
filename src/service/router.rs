use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    KeyedUri, Result,
};

use helium_proto::services::{
    self,
    router::{PacketRouterClient, PacketRouterPacketDownV1, PacketRouterPacketUpV1},
    Channel, Endpoint,
};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

// type RouterClient = services::router::RouterClient<Channel>;

type PacketClient = services::router::PacketRouterClient<Channel>;

type PacketSender = mpsc::Sender<PacketRouterPacketUpV1>;
type PacketReceiver = tonic::Streaming<PacketRouterPacketDownV1>;

#[derive(Debug)]
pub struct RouterService {
    pub uri: KeyedUri,
    packet_router_client: PacketRouterClient<Channel>,
    conduit: Option<(PacketSender, PacketReceiver)>,
}

pub const CONDUIT_CAPACITY: usize = 50;

impl RouterService {
    pub fn new(keyed_uri: KeyedUri) -> Result<Self> {
        let packet_channel = Endpoint::from(keyed_uri.uri.clone())
            .timeout(RPC_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .connect_lazy();
        Ok(Self {
            uri: keyed_uri,
            packet_router_client: PacketClient::new(packet_channel),
            conduit: None,
        })
    }

    pub async fn route(&mut self, msg: PacketRouterPacketUpV1) -> Result<()> {
        self.send(msg).await?;
        Ok(())
    }

    pub async fn send(&mut self, msg: PacketRouterPacketUpV1) -> Result {
        if self.conduit.is_none() {
            self.conduit = Some(self.mk_conduit().await?);
        }

        let (tx, _) = self.conduit.as_ref().unwrap();
        Ok(tx.send(msg).await?)
    }

    pub async fn mk_conduit(&mut self) -> Result<(PacketSender, PacketReceiver)> {
        let (tx, client_rx) = mpsc::channel(CONDUIT_CAPACITY);
        let rx = self
            .packet_router_client
            .msg(ReceiverStream::new(client_rx))
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
            Ok(Some(msg)) => Ok(Some(msg)),
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
}
