use crate::{gateway::Gateway, result::Result, settings::Settings};
use log::info;

pub async fn run(shutdown: &triggered::Listener, settings: &Settings) -> Result {
    let mut gateway = Gateway::new(&settings).await?;
    // TODO: Concurrently run the udp listener, updater, and listen for the
    // shutdown signal.
    gateway.run(shutdown.clone()).await?;
    info!("Shutting down server");
    Ok(())
}
