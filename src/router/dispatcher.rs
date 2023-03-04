use crate::{
    gateway, packet_router, region_watcher,
    router::{self, RouterClient, Routing},
    service::{self, gateway::GatewayService},
    Error, KeyedUri, Keypair, Packet, RegionParams, Result, Settings,
};
use exponential_backoff::Backoff;
use futures::{
    task::{Context, Poll},
    TryFutureExt,
};
use std::{
    collections::HashMap,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{task::JoinHandle, time};
use tokio_stream::{self, StreamExt};
use tracing::{debug, info, warn};

pub type Message = packet_router::Message;
pub type MessageReceiver = packet_router::MessageReceiver;
pub type MessageSender = packet_router::MessageSender;

pub struct Dispatcher {
    keypair: Arc<Keypair>,
    messages: MessageReceiver,
    region_params: RegionParams,
    region_watch: region_watcher::MessageReceiver,
    transmit: gateway::MessageSender,
    seed_gateways: Vec<KeyedUri>,
    routing_height: u64,
    max_packets: u16,
    gateway_retry: u32,
    routers: HashMap<RouterKey, RouterEntry>,
    default_routers: Option<Vec<KeyedUri>>,
}

#[derive(PartialEq, Eq, Hash)]
struct RouterKey {
    oui: u32,
    uri: KeyedUri,
}

#[derive(Debug)]
struct RouterEntry {
    routing: Routing,
    dispatch: router::client::MessageSender,
    join_handle: JoinHandle<Result>,
}

const GATEWAY_BACKOFF_RETRIES: u32 = 10;
const GATEWAY_BACKOFF_MIN_WAIT: Duration = Duration::from_secs(5);
const GATEWAY_BACKOFF_MAX_WAIT: Duration = Duration::from_secs(1800); // 30 minutes

const GATEWAY_CHECK_INTERVAL: Duration = Duration::from_secs(900); // 15 minutes
const GATEWAY_MAX_BLOCK_AGE: Duration = Duration::from_secs(1800); // 30 minutes

type RoutingStream = service::gateway::Streaming;

impl Dispatcher {
    // Allow mutable key type for HashMap with Uri in the key
    #[allow(clippy::mutable_key_type)]
    pub fn new(
        settings: &Settings,
        messages: MessageReceiver,
        region_watch: region_watcher::MessageReceiver,
        transmit: gateway::MessageSender,
    ) -> Self {
        let seed_gateways = settings.gateways.clone();
        let routers = HashMap::with_capacity(5);
        let default_routers = settings.routers.clone();
        let max_packets = settings.router.queue;
        let region_params = region_watcher::current_value(&region_watch);
        Self {
            keypair: settings.keypair.clone(),
            messages,
            region_params,
            region_watch,
            transmit,
            seed_gateways,
            routers,
            routing_height: 0,
            default_routers,
            max_packets,
            gateway_retry: 0,
        }
    }

    pub async fn run(&mut self, shutdown: &triggered::Listener) -> Result {
        info!("starting");

        if let Some(default_routers) = &self.default_routers {
            for default_router in default_routers {
                info!(
                    pubkey = %default_router.pubkey,
                    uri = %default_router.uri,
                    "default router"
                );
            }
        }

        let gateway_backoff = Backoff::new(
            GATEWAY_BACKOFF_RETRIES,
            GATEWAY_BACKOFF_MIN_WAIT,
            GATEWAY_BACKOFF_MAX_WAIT,
        );
        loop {
            if shutdown.is_triggered() {
                // Prevent unneeded seed reselection
                return Ok(());
            }
            // Select seed
            let seed_gateway = GatewayService::select_seed(&self.seed_gateways)?;
            info!(
                pubkey = %seed_gateway.uri.pubkey,
                uri = %seed_gateway.uri.uri,
                "seed gateway"
            );

            tokio::select! {
                _ = shutdown.clone() => {
                    info!("shutting down");
                    return Ok(())
                },
                region_change = self.region_watch.changed() => match region_change {
                    Ok(()) => self.handle_region_params_update().await,
                    Err(_) => warn!("region watch disconnected"),
                },
                // Try to select a random validator from the seed and fetch the needed streams
                gateway = Self::select_gateway(seed_gateway, shutdown)
                    .and_then(|service | Self::setup_routing_stream(service, self.routing_height))
                     => match gateway {
                        Ok(Some((service, gateway_streams))) => {
                            self.run_with_gateway(service, gateway_streams,  shutdown.clone())
                                .await?;
                            },
                        Ok(None) =>
                            return Ok(()),
                        Err(_err) => ()
                    }
            }

            self.prepare_gateway_change(&gateway_backoff, shutdown.clone())
                .await;
        }
    }

    async fn select_gateway(
        mut seed_gateway: GatewayService,
        shutdown: &triggered::Listener,
    ) -> Result<Option<GatewayService>> {
        match seed_gateway.random_new(5, shutdown.clone()).await {
            Ok(result) => Ok(result),
            Err(err) => {
                warn!(
                    pubkey = %seed_gateway.uri.pubkey,
                    uri = %seed_gateway.uri.uri,
                    %err,
                    "failed to select gateway"
                );
                Err(err)
            }
        }
    }

    async fn setup_routing_stream(
        gateway: Option<GatewayService>,
        routing_height: u64,
    ) -> Result<Option<(GatewayService, RoutingStream)>> {
        if gateway.is_none() {
            return Ok(None);
        }
        let mut gateway = gateway.unwrap();
        match gateway.routing(routing_height).await {
            Ok(routing) => Ok(Some((gateway, routing))),
            Err(err) => {
                warn!(
                    pubkey = %gateway.uri.pubkey,
                    uri = %gateway.uri.uri,
                    %err,
                    "failed to set up gateway routing stream"
                );
                Err(err)
            }
        }
    }

    async fn run_with_gateway(
        &mut self,
        mut gateway: GatewayService,
        mut routing: RoutingStream,
        shutdown: triggered::Listener,
    ) -> Result {
        info!(
            pubkey = %gateway.uri.pubkey,
            uri = %gateway.uri.uri,
            "using gateway"
        );
        // Initialize liveness check for gateway
        let mut gateway_check = time::interval(GATEWAY_CHECK_INTERVAL);
        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!("shutting down");
                    return Ok(())
                },
                region_change = self.region_watch.changed() => match region_change {
                    Ok(()) => self.handle_region_params_update().await,
                    Err(_) => warn!("region watch disconnected"),
                },
                gateway_msg = routing.next() => match gateway_msg {
                    Some(Ok(gateway_message)) => self.handle_routing_update(&gateway_message, &shutdown).await,
                    Some(Err(err)) =>  {
                        warn!(%err, "gateway routing stream failure");
                        return Ok(())
                    },
                    None => {
                        warn!("gateway streams closed");
                        return Ok(());
                }
                },
                _ = gateway_check.tick() => match self.check_gateway(&mut gateway).await {
                    Ok(()) => {
                        self.gateway_retry = 0
                    },
                    Err(err) => {
                        warn!("gateway check error: {err}");
                        return Ok(())
                    }
                },
                message = self.messages.recv() => match message {
                    Some(Message::Uplink{packet, received}) =>
                        self.handle_uplink(packet, received).await,
                    None => warn!("ignoring closed message channel"),
                },
            }
        }
    }

    async fn check_gateway(&mut self, gateway: &mut GatewayService) -> Result {
        let (_, block_age) = gateway.height().await?;
        info!( 
            pubkey = %gateway.uri.pubkey,
            block_age = block_age, 
            "checking gateway");
        if block_age > GATEWAY_MAX_BLOCK_AGE.as_secs() {
            return Err(Error::gateway_service_check(
                block_age,
                GATEWAY_MAX_BLOCK_AGE.as_secs(),
            ));
        }
        Ok(())
    }

    async fn prepare_gateway_change(
        &mut self,
        backoff: &Backoff,
        shutdown: triggered::Listener,
    ) {
        // Check if shutdown trigger already happened
        if shutdown.is_triggered() {
            return;
        }

        // Reset routing and region heigth for the next gateway
        self.routing_height = 0;

        // Use backof to sleep exponentially longer
        self.gateway_retry += 1;
        let sleep = backoff
            .next(self.gateway_retry)
            .unwrap_or(GATEWAY_BACKOFF_MAX_WAIT);

        // Select over either shutdown or sleep, and handle messages that don't
        // require a gateway
        info!("selecting new gateway in {}s", sleep.as_secs());
        tokio::select! {
                _ = shutdown => {},
                _ = time::sleep(sleep) => {},
                region_change = self.region_watch.changed() => match region_change {
                    Ok(()) => self.handle_region_params_update().await,
                    Err(_) => warn!("region watch disconnected"),
                },
                message = self.messages.recv() => match message {
                    Some(Message::Uplink{packet, received}) =>
                        self.handle_uplink(packet, received).await,
                    None => warn!("ignoring closed message channel"),
                },
        }
    }

    async fn handle_uplink(&self, packet: Packet, received: Instant) {
        let mut handled = false;
        for router_entry in self.routers.values() {
            if router_entry.routing.matches_routing_info(packet.routing()) {
                match router_entry.dispatch.uplink(packet.clone(), received).await {
                    Ok(()) => (),
                    Err(err) => warn!(%err, "ignoring router dispatch error"),
                }
                handled = true;
            }
        }
        if !handled {
            if let Some(default_routers) = &self.default_routers {
                for (router_key, router_entry) in &self.routers {
                    if default_routers.contains(&router_key.uri) {
                        debug!("sending to default router");
                        let _ = router_entry.dispatch.uplink(packet.clone(), received).await;
                    }
                }
            }
        }
    }

    async fn handle_region_params_update(&mut self) {
        self.region_params = region_watcher::current_value(&self.region_watch);
        info!(regioon = %self.region_params, "region updated");
    }

    async fn handle_routing_update<R: service::gateway::Response>(
        &mut self,
        response: &R,
        shutdown: &triggered::Listener,
    ) {
        let update_height = response.height();
        let current_height = self.routing_height;
        if update_height <= self.routing_height {
            warn!(
                routing_height = current_height,
                "routing returned invalid height {update_height}",
            );
            return;
        }
        let routing_protos = match response.routings() {
            Ok(v) => v,
            Err(err) => {
                warn!(%err, "failed to decode routing");
                return;
            }
        };
        let mut proto_stream = tokio_stream::iter(routing_protos.iter());
        while let Some(proto) = proto_stream.next().await {
            match Routing::from_proto(proto) {
                Ok(routing) => {
                    self.handle_oui_routing_update(&routing, shutdown)
                        .await
                }
                Err(err) => warn!(%err, "failed to parse routing"),
            }
        }
        self.routing_height = update_height;
        info!(routing_height = self.routing_height, "routing height updated");
    }

    #[allow(clippy::map_entry)]
    async fn handle_oui_routing_update(
        &mut self,
        routing: &Routing,
        shutdown: &triggered::Listener,
    ) {
        let mut uris = tokio_stream::iter(routing.uris.iter());
        while let Some(uri) = uris.next().await {
            let key = RouterKey {
                oui: routing.oui,
                uri: uri.to_owned(),
            };
            // We have to allow clippy::map_entry above since we need to borrow
            // immutable before borrowing as mutable to insert
            match self.routers.get_mut(&key) {
                Some(router_entry) => router_entry.routing = routing.clone(),
                None => match self
                    .start_router(shutdown.clone(), routing.clone(), uri.clone())
                    .await
                {
                    Ok(router_entry) => {
                        self.routers.insert(key, router_entry);
                    }
                    Err(err) => {
                        warn!(%err, "faild to construct router");
                    }
                },
            }
        }
        // Remove any routers that are not in the new oui uri list
        let mut removables = Vec::with_capacity(self.routers.len());
        self.routers.retain(|key, entry| {
            if key.oui == routing.oui && !entry.routing.contains_uri(&key.uri) {
                // Router will be removed from the map. The router is expected
                // to stop itself when it receives the stop message
                info!(
                    oui = key.oui,
                    uri = %key.uri.uri,
                    "removing router"                    
                );
                removables.push(entry.dispatch.clone());
                return false;
            }
            true
        });
        for removable in removables {
            removable.stop().await;
        }
    }

    async fn start_router(
        &self,
        shutdown: triggered::Listener,
        routing: Routing,
        uri: KeyedUri,
    ) -> Result<RouterEntry> {
        let (client_tx, client_rx) = router::client::message_channel(10);
        let mut client = RouterClient::new(
            routing.oui,
            self.region_watch.clone(),
            uri,
            self.transmit.clone(),
            self.keypair.clone(),
            self.max_packets,
        )
        .await?;
        let join_handle =
            tokio::spawn(async move { client.run(client_rx, shutdown).await });
        Ok(RouterEntry {
            routing,
            dispatch: client_tx,
            join_handle,
        })
    }
}

impl std::future::Future for RouterEntry {
    type Output = std::result::Result<Result, tokio::task::JoinError>;

    fn poll(
        mut self: Pin<&mut Self>,
        cxt: &mut Context<'_>,
    ) -> Poll<<Self as futures::Future>::Output> {
        Pin::new(&mut self.join_handle).poll(cxt)
    }
}
