use crate::*;

/// Run the gateway service
#[derive(Debug, clap::Args)]
pub struct Cmd {}

impl Cmd {
    pub async fn run(&self, shutdown: &triggered::Listener, settings: Settings) -> Result {
        server::run(shutdown, &settings).await
    }
}
