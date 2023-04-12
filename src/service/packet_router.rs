use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    error::DecodeError,
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    sign, Error, Keypair, Result,
};

use helium_proto::{
    services::{
        router::{
            envelope_down_v1, envelope_up_v1, EnvelopeDownV1, EnvelopeUpV1, PacketRouterClient,
            PacketRouterPacketDownV1, PacketRouterPacketUpV1, PacketRouterRegisterV1,
        },
        Channel, Endpoint,
    },
    Message,
};

use http::Uri;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

type PacketClient = PacketRouterClient<Channel>;

type PacketSender = mpsc::Sender<EnvelopeUpV1>;
type PacketReceiver = tonic::Streaming<EnvelopeDownV1>;

// The router service maintains a re-connectable connection to a remote packet
// router. The service will connect when (re)connect or a packet send is
// attempted. It will ensure that the register rpc is called on the constructed
// connection before a packet is sent.
#[derive(Debug)]
pub struct PacketRouterService {
    pub uri: Uri,
    conduit: Option<PacketRouterConduit>,
    keypair: Arc<Keypair>,
}

/// A router conduit is the tx/rx stream pair for the `route` rpc on the
/// `packet_router` service. It does not connect on construction but on the
/// first messsage sent.
#[derive(Debug)]
struct PacketRouterConduit {
    tx: PacketSender,
    rx: PacketReceiver,
}

pub const CONDUIT_CAPACITY: usize = 50;

/// The time between TCP keepalive messages to keep the connection to the packet
/// router open. Some load balancer disconnect after a number of seconds. AWS
/// NLBs are hardcoded to 350s so we pick a slightly shorter timeframe to send
/// keepalives
pub const TCP_KEEP_ALIVE_DURATION: std::time::Duration = std::time::Duration::from_secs(300);

impl PacketRouterConduit {
    async fn new(uri: Uri) -> Result<Self> {
        let endpoint = Endpoint::from(uri)
            .timeout(RPC_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_keepalive(Some(TCP_KEEP_ALIVE_DURATION))
            .connect_lazy();
        let mut client = PacketClient::new(endpoint);
        let (tx, client_rx) = mpsc::channel(CONDUIT_CAPACITY);
        let rx = client
            .route(ReceiverStream::new(client_rx))
            .await?
            .into_inner();
        Ok(Self { tx, rx })
    }

    async fn recv(&mut self) -> Result<Option<PacketRouterPacketDownV1>> {
        match self.rx.message().await {
            Ok(Some(msg)) => match msg.data {
                Some(envelope_down_v1::Data::Packet(packet)) => Ok(Some(packet)),
                None => Err(DecodeError::invalid_envelope()),
            },
            Ok(None) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    async fn send(&mut self, msg: PacketRouterPacketUpV1) -> Result {
        let msg = EnvelopeUpV1 {
            data: Some(envelope_up_v1::Data::Packet(msg)),
        };
        Ok(self.tx.send(msg).await?)
    }

    async fn register(&mut self, keypair: Arc<Keypair>) -> Result {
        let mut msg = PacketRouterRegisterV1 {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(Error::from)?
                .as_millis() as u64,
            gateway: keypair.public_key().into(),
            signature: vec![],
        };
        msg.signature = sign(keypair.clone(), msg.encode_to_vec()).await?;
        let msg = EnvelopeUpV1 {
            data: Some(envelope_up_v1::Data::Register(msg)),
        };
        Ok(self.tx.send(msg).await?)
    }
}

impl PacketRouterService {
    pub fn new(uri: Uri, keypair: Arc<Keypair>) -> Self {
        Self {
            uri,
            conduit: None,
            keypair,
        }
    }

    pub async fn send(&mut self, msg: PacketRouterPacketUpV1) -> Result {
        if self.conduit.is_none() {
            self.connect().await?;
        }
        // Unwrap since the above connect early exits if no conduit is created
        match self.conduit.as_mut().unwrap().send(msg).await {
            Ok(()) => Ok(()),
            other => {
                self.disconnect();
                other
            }
        }
    }

    pub async fn recv(&mut self) -> Result<Option<PacketRouterPacketDownV1>> {
        // Since recv is usually called from a select loop we don't try a
        // connect every time it is called since the rate for attempted
        // connections in failure setups would be as high as the loop rate of
        // the caller. This relies on either a reconnect attempt or a packet
        // send at a later time to reconnect the conduit.
        if self.conduit.is_none() {
            futures::future::pending::<()>().await;
            return Ok(None);
        }
        match self.conduit.as_mut().unwrap().recv().await {
            Ok(msg) if msg.is_some() => Ok(msg),
            other => {
                self.disconnect();
                other
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.conduit = None;
    }

    pub async fn connect(&mut self) -> Result {
        let mut conduit = PacketRouterConduit::new(self.uri.clone()).await?;
        conduit.register(self.keypair.clone()).await?;
        self.conduit = Some(conduit);
        Ok(())
    }

    pub async fn reconnect(&mut self) -> Result {
        self.disconnect();
        self.connect().await
    }

    pub fn is_connected(&self) -> bool {
        self.conduit.is_some()
    }
}
