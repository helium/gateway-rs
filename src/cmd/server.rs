use crate::*;
use slog::Logger;
use structopt::StructOpt;

/// Run the gateway service
#[derive(Debug, StructOpt)]
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
