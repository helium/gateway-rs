use crate::*;
use slog::Logger;

/// Run the gateway service
#[derive(Debug, clap::Args)]
pub struct Cmd {}

impl Cmd {
    pub async fn run(
        &self,
        shutdown: &triggered::Listener,
        settings: Settings,
        logger: &Logger,
    ) -> Result {
        server::run(shutdown, &settings, logger).await
    }
}
