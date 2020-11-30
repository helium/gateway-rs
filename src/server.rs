use crate::{
    error::Result,
    gateway::Gateway,
    settings::{self, Settings},
    updater::Updater,
};
use log::info;

pub async fn run(shutdown: &triggered::Listener, settings: &Settings) -> Result {
    let mut gateway = Gateway::new(&settings).await?;
    let updater = Updater::new(&settings)?;

    info!(
        "starting server: {} id: {}",
        settings::version(),
        settings.keypair
    );
    tokio::try_join!(gateway.run(shutdown.clone()), updater.run(shutdown.clone())).map(|_| ())
}
