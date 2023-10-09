use crate::{
    service::{CONNECT_TIMEOUT, RPC_TIMEOUT},
    Keypair, Result,
};
use helium_proto::services::{Channel, Endpoint};
use http::Uri;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

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
    pub fn new(uri: Uri, client: C, keypair: Arc<Keypair>) -> Self {
        Self {
            uri,
            conduit: None,
            keypair,
            client,
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

    pub async fn recv(&mut self) -> Result<Option<D>> {
        // Since recv is usually called from a select loop we don't try a
        // connect every time it is called since the rate for attempted
        // connections in failure setups would be as high as the loop rate of
        // the caller. This relies on either a reconnect attempt or a message
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
        self.conduit.is_some()
    }
}
