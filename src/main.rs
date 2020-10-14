use anyhow::Result;
use gateway_rs::{server, settings};
use std::path::PathBuf;
use structopt::StructOpt;
use tokio::signal;
use tracing::info;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Debug, StructOpt)]
#[structopt(name = "gateway", version = env!("CARGO_PKG_VERSION"), about = "Helium Light Gateway")]
pub struct Cli {
    /// Config file to load. Defaults to "config/default"
    #[structopt(short = "c")]
    config: Option<PathBuf>,
}

#[tokio::main]
pub async fn main() -> Result<()> {
    // Enable logging
    let fmt_layer = fmt::layer().with_target(false);
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    let cli = Cli::from_args();
    // Load settings
    let settings = settings::Settings::new(cli.config)?;

    let (shutdown_trigger, shutdown_listener) = triggered::trigger();
    info!("Starting Server");
    tokio::spawn(async move {
        let _ = signal::ctrl_c().await;
        shutdown_trigger.trigger();
    });
    server::run(&settings, shutdown_listener).await
}
