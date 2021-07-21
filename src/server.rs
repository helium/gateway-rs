use crate::*;
use gateway::Gateway;
use router::Dispatcher;
use slog::{info, Logger};
use tokio::sync::mpsc;
use updater::Updater;

pub async fn run(shutdown: &triggered::Listener, settings: &Settings, logger: &Logger) -> Result {
    let (uplink_sender, uplink_receiver) = mpsc::channel(20);
    let (downlink_sender, downlink_receiver) = mpsc::channel(10);
    let mut dispatcher =
        Dispatcher::new(shutdown.clone(), downlink_sender, uplink_receiver, settings)?;
    let mut gateway = Gateway::new(uplink_sender, downlink_receiver, settings).await?;
    let updater = Updater::new(settings)?;
    info!(logger,
        "starting server";
        "version" => settings::version().to_string(),
        "key" => settings.keypair.public_key().to_string(),
    );
    tokio::try_join!(
        gateway.run(shutdown.clone(), logger),
        dispatcher.run(logger),
        updater.run(shutdown.clone(), logger)
    )
    .map(|_| ())
}
