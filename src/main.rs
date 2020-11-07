use env_logger::Env;
use gateway_rs::{cmd, result::Result, settings::Settings};
use std::path::PathBuf;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "gateway", version = env!("CARGO_PKG_VERSION"), about = "Helium Light Gateway")]
pub struct Cli {
    /// Config file to load. Defaults to "config/default"
    #[structopt(short = "c")]
    config: Option<PathBuf>,

    #[structopt(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, StructOpt)]
pub enum Cmd {
    Key(cmd::key::Cmd),
    Server(cmd::server::Cmd),
}

#[tokio::main]
pub async fn main() -> Result {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let cli = Cli::from_args();

    let settings = Settings::new(cli.config.clone())?;

    let (shutdown_trigger, shutdown_listener) = triggered::trigger();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        shutdown_trigger.trigger();
    });
    run(cli, settings, &shutdown_listener).await
}

pub async fn run(cli: Cli, settings: Settings, shutdown_listener: &triggered::Listener) -> Result {
    match cli.cmd {
        Cmd::Key(cmd) => cmd.run(settings).await,
        Cmd::Server(cmd) => cmd.run(shutdown_listener, settings).await,
    }
}
