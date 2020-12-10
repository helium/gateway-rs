use crate::{
    error::Result,
    gateway::Gateway,
    settings::{self, Settings},
    updater::Updater,
};
use slog::{info, Logger};

pub async fn run(shutdown: &triggered::Listener, settings: &Settings, logger: &Logger) -> Result {
    let mut gateway = Gateway::new(&settings).await?;
    let updater = Updater::new(&settings)?;
    info!(logger,
        "starting server";
        "version" => settings::version().to_string(),
        "key" => settings.keypair.to_string(),
    );
    tokio::try_join!(
        gateway.run(shutdown.clone(), logger),
        updater.run(shutdown.clone(), logger)
    )
    .map(|_| ())
}
