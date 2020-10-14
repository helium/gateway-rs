use crate::{gateway::Gateway, settings::Settings};
use anyhow::Result;
use tracing::info;

pub async fn run(settings: &Settings, shutdown: triggered::Listener) -> Result<()> {
    let mut gateway = Gateway::new(settings).await?;
    // Concurrently run the udp listener, updater, and listen for the shutdown
    // signal.
    gateway.run(shutdown.clone()).await?;
    info!("Shutting down server");
    Ok(())
}
