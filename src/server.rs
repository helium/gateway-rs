use crate::*;
use api::LocalServer;
use gateway;
use router::{dispatcher, Dispatcher};
use slog::{info, Logger};
use updater::Updater;

pub async fn run(shutdown: &triggered::Listener, settings: &Settings, logger: &Logger) -> Result {
    let (gateway_tx, gateway_rx) = gateway::message_channel(10);
    let (dispatcher_tx, dispatcher_rx) = dispatcher::message_channel(20);
    let mut dispatcher = Dispatcher::new(dispatcher_rx, gateway_tx, settings)?;
    let mut gateway = gateway::Gateway::new(dispatcher_tx.clone(), gateway_rx, settings).await?;
    let updater = Updater::new(settings)?;
    let api = LocalServer::new(dispatcher_tx, settings)?;
    info!(logger,
        "starting server";
        "version" => settings::version().to_string(),
        "key" => settings.keypair.public_key().to_string(),
    );
    tokio::try_join!(
        gateway.run(shutdown.clone(), logger),
        dispatcher.run(shutdown.clone(), logger),
        updater.run(shutdown.clone(), logger),
        api.run(shutdown.clone(), logger),
    )
    .map(|_| ())
}
