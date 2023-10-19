use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Error, Keypair, PublicKey, Result, Sign,
};
use futures::TryFutureExt;
use helium_proto::services::{Channel, Endpoint};
use http::Uri;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};

/// The time between TCP keepalive messages to keep the connection to the packet
/// router open. Some load balancer disconnect after a number of seconds. AWS
/// NLBs are hardcoded to 350s so we pick a slightly shorter timeframe to send
/// keepalives
pub const TCP_KEEP_ALIVE_DURATION: std::time::Duration = std::time::Duration::from_secs(300);
pub const CONDUIT_CAPACITY: usize = 50;

/// A conduit service maintains a re-connectable connection to a remote service.
#[derive(Debug)]
pub struct ConduitService<U, D, C: ConduitClient<U, D>> {
    pub uri: Uri,
    module: &'static str,
    session_keypair: Option<Arc<Keypair>>,
    conduit: Option<Conduit<U, D>>,
    keypair: Arc<Keypair>,
    client: C,
}

#[derive(Debug)]
struct Conduit<U, D> {
    tx: mpsc::Sender<U>,
    rx: tonic::Streaming<D>,
}

#[tonic::async_trait]
pub trait ConduitClient<U, D> {
    async fn init(
        &mut self,
        endpoint: Channel,
        tx: mpsc::Sender<U>,
        client_rx: ReceiverStream<U>,
        keypair: Arc<Keypair>,
    ) -> Result<tonic::Streaming<D>>;

    async fn mk_session_init(
        &self,
        nonce: &[u8],
        session_key: &PublicKey,
        keypair: Arc<Keypair>,
    ) -> Result<U>;
}

impl<U, D> Conduit<U, D> {
    async fn new<C: ConduitClient<U, D>>(
        uri: Uri,
        client: &mut C,
        keypair: Arc<Keypair>,
    ) -> Result<Self> {
        let endpoint = Endpoint::from(uri)
            .timeout(RPC_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_keepalive(Some(TCP_KEEP_ALIVE_DURATION))
            .connect_lazy();
        let (tx, client_rx) = mpsc::channel(CONDUIT_CAPACITY);
        let rx = client
            .init(
                endpoint,
                tx.clone(),
                ReceiverStream::new(client_rx),
                keypair,
            )
            .await?;
        Ok(Self { tx, rx })
    }

    async fn recv(&mut self) -> Result<Option<D>> {
        Ok(self.rx.message().await?)
    }

    async fn send(&mut self, msg: U) -> Result {
        Ok(self.tx.send(msg).await?)
    }
}

impl<U, D, C: ConduitClient<U, D>> ConduitService<U, D, C> {
    pub fn new(module: &'static str, uri: Uri, client: C, keypair: Arc<Keypair>) -> Self {
        Self {
            uri,
            module,
            keypair,
            client,
            conduit: None,
            session_keypair: None,
        }
    }

    pub async fn send(&mut self, msg: U) -> Result {
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

    pub async fn recv(&mut self) -> Result<D> {
        // Since recv is usually called from a select loop we don't try a
        // connect every time it is called since the rate for attempted
        // connections in failure setups would be as high as the loop rate of
        // the caller. This relies on either a reconnect attempt or a message
        // send at a later time to reconnect the conduit.
        if self.conduit.is_none() {
            futures::future::pending::<()>().await;
            return Err(Error::no_stream());
        }
        match self.conduit.as_mut().unwrap().recv().await {
            Ok(Some(msg)) => Ok(msg),
            Ok(None) => {
                self.disconnect();
                Err(Error::no_stream())
            }
            Err(err) => {
                self.disconnect();
                Err(err)
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.conduit = None;
        self.session_keypair = None;
    }

    pub async fn connect(&mut self) -> Result {
        let conduit =
            Conduit::new(self.uri.clone(), &mut self.client, self.keypair.clone()).await?;
        self.conduit = Some(conduit);
        Ok(())
    }

    pub async fn reconnect(&mut self) -> Result {
        self.disconnect();
        self.connect().await
    }

    pub fn is_connected(&self) -> bool {
        self.conduit.is_some() && self.session_keypair.is_some()
    }

    pub fn gateway_key(&self) -> &PublicKey {
        self.keypair.public_key()
    }

    pub fn session_key(&self) -> Option<&PublicKey> {
        self.session_keypair.as_ref().map(|k| k.public_key())
    }

    pub fn session_keypair(&self) -> Option<Arc<Keypair>> {
        self.session_keypair.clone()
    }

    pub async fn session_sign<M: Sign>(&self, msg: &mut M) -> Result {
        if let Some(keypair) = self.session_keypair.as_ref() {
            msg.sign(keypair.clone()).await?;
            Ok(())
        } else {
            Err(Error::no_session())
        }
    }

    pub async fn session_init(&mut self, nonce: &[u8]) -> Result {
        let session_keypair = Arc::new(Keypair::new());
        let session_key = session_keypair.public_key();
        let module: &'static str = self.module;
        let msg = self
            .client
            .mk_session_init(nonce, session_key, self.keypair.clone())
            .await?;
        self.send(msg)
            .inspect_err(|err| warn!(module, %err, "failed to initialize session"))
            .await?;
        self.session_keypair = Some(session_keypair.clone());
        info!(module, %session_key, "initialized session");
        Ok(())
    }
}
