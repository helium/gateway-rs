use crate::{
    api::LocalServer,
    beaconer, gateway, packet_router, region_watcher,
    settings::{self, Settings},
    Result,
};
use tracing::info;

#[tracing::instrument(skip_all)]
pub async fn run(shutdown: &triggered::Listener, settings: &Settings) -> Result {
    let (gateway_tx, gateway_rx) = gateway::message_channel();
    let (router_tx, router_rx) = packet_router::message_channel();
    let (beacon_tx, beacon_rx) = beaconer::message_channel();

    let mut region_watcher = region_watcher::RegionWatcher::new(settings);
    let region_rx = region_watcher.watcher();

    let mut beaconer =
        beaconer::Beaconer::new(settings, beacon_rx, region_rx.clone(), gateway_tx.clone());

    let mut router = packet_router::PacketRouter::new(settings, router_rx, gateway_tx.clone());

    let mut gateway = gateway::Gateway::new(
        settings,
        gateway_rx,
        region_rx.clone(),
        router_tx.clone(),
        beacon_tx,
    )
    .await?;
    let api = LocalServer::new(region_rx.clone(), router_tx.clone(), settings)?;
    info!(
        version = %settings::version().to_string(),
        key = %settings.keypair.public_key().to_string(),
        "starting server",
    );
    tokio::try_join!(
        region_watcher.run(shutdown),
        beaconer.run(shutdown),
        gateway.run(shutdown),
        router.run(shutdown),
        api.run(shutdown),
    )
    .map(|_| ())
}
