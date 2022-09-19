use crate::*;
use api::LocalServer;
use beaconing;
use gateway;
use router::{dispatcher, Dispatcher};
use slog::{info, Logger};
use updater::Updater;

pub async fn run(shutdown: &triggered::Listener, settings: &Settings, logger: &Logger) -> Result {
    let (gateway_tx, gateway_rx) = gateway::message_channel(10);
    let (dispatcher_tx, dispatcher_rx) = dispatcher::message_channel(20);
    let (beaconing_tx, beaconing_rx) = beaconing::message_channel(10);
    let mut beaconer = beaconing::Beaconer::new(settings, gateway_tx.clone(), beaconing_rx, logger);
    let mut dispatcher = Dispatcher::new(dispatcher_rx, gateway_tx, settings)?;
    let mut gateway =
        gateway::Gateway::new(dispatcher_tx.clone(), gateway_rx, beaconing_tx, settings).await?;
    let updater = Updater::new(settings)?;
    let api = LocalServer::new(dispatcher_tx, settings)?;
    info!(logger,
        "starting server";
        "version" => settings::version().to_string(),
        "key" => settings.keypair.public_key().to_string(),
    );
    tokio::try_join!(
        beaconer.run(shutdown.clone()),
        gateway.run(shutdown.clone(), logger),
        dispatcher.run(shutdown.clone(), logger),
        updater.run(shutdown.clone(), logger),
        api.run(shutdown.clone(), logger),
    )
    .map(|_| ())
}
