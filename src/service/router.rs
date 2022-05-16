use crate::{service::CONNECT_TIMEOUT, KeyedUri, Result};
use helium_proto::{
    services::{self, Channel, Endpoint},
    BlockchainStateChannelMessageV1,
};
use tokio::{sync::mpsc, time::Duration};
use tokio_stream::wrappers::ReceiverStream;

type RouterClient = services::router::RouterClient<Channel>;
type StateChannelClient = services::router::StateChannelClient<Channel>;

#[derive(Debug)]
pub struct RouterService {
    pub uri: KeyedUri,
    router_client: RouterClient,
    state_channel_client: StateChannelClient,
}

#[derive(Debug)]
pub struct StateChannelService {
    client: StateChannelClient,
    conduit: Option<(
        mpsc::Sender<BlockchainStateChannelMessageV1>,
        tonic::Streaming<BlockchainStateChannelMessageV1>,
    )>,
}

pub const CONDUIT_CAPACITY: usize = 50;

impl StateChannelService {
    pub async fn send(&mut self, msg: BlockchainStateChannelMessageV1) -> Result {
        if self.conduit.is_none() {
            self.conduit = Some(self.mk_conduit().await?)
        }
        let (tx, _) = self.conduit.as_ref().unwrap();
        Ok(tx.send(msg).await?)
    }

    pub fn capacity(&self) -> usize {
        self.conduit
            .as_ref()
            .map(|(tx, _)| tx.capacity())
            .unwrap_or(CONDUIT_CAPACITY)
    }

    pub async fn message(&mut self) -> Result<Option<BlockchainStateChannelMessageV1>> {
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

    pub async fn connect(&mut self) -> Result {
        if self.conduit.is_none() {
            self.conduit = Some(self.mk_conduit().await?);
        }
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.conduit.is_some()
    }

    pub fn disconnect(&mut self) {
        self.conduit = None;
    }

    pub async fn reconnect(&mut self) -> Result {
        self.disconnect();
        self.connect().await
    }

    pub async fn mk_conduit(
        &mut self,
    ) -> Result<(
        mpsc::Sender<BlockchainStateChannelMessageV1>,
        tonic::Streaming<BlockchainStateChannelMessageV1>,
    )> {
        let (tx, client_rx) = mpsc::channel(CONDUIT_CAPACITY);
        let rx = self
            .client
            .msg(ReceiverStream::new(client_rx))
            .await?
            .into_inner();
        Ok((tx, rx))
    }
}

impl RouterService {
    pub fn new(keyed_uri: KeyedUri) -> Result<Self> {
        let router_channel = Endpoint::from(keyed_uri.uri.clone())
            .timeout(Duration::from_secs(CONNECT_TIMEOUT))
            .connect_lazy();
        let state_channel = router_channel.clone();
        Ok(Self {
            uri: keyed_uri,
            router_client: RouterClient::new(router_channel),
            state_channel_client: StateChannelClient::new(state_channel),
        })
    }

    pub async fn route(
        &mut self,
        msg: BlockchainStateChannelMessageV1,
    ) -> Result<BlockchainStateChannelMessageV1> {
        Ok(self.router_client.route(msg).await?.into_inner())
    }

    pub fn state_channel(&mut self) -> Result<StateChannelService> {
        Ok(StateChannelService {
            client: self.state_channel_client.clone(),
            conduit: None,
        })
    }
}
