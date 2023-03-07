use clap::Parser;
use gateway_rs::{cmd, error::Result, settings::Settings};
use std::path::PathBuf;
use tokio::{io::AsyncReadExt, signal, time::Duration};
use tracing::{debug, error, Level};
use tracing_subscriber::prelude::*;

#[derive(Debug, Parser)]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(name = env!("CARGO_BIN_NAME"))]
/// Helium Gateway
pub struct Cli {
    /// Configuration file to use
    #[arg(short = 'c', default_value = "/etc/helium_gateway/settings.toml")]
    config: PathBuf,

    /// Monitor stdin and terminate when stdin closes.
    #[arg(long)]
    stdin: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, clap::Subcommand)]
pub enum Cmd {
    Key(cmd::key::Cmd),
    Info(cmd::info::Cmd),
    Server(cmd::server::Cmd),
    Add(Box<cmd::add::Cmd>),
}

fn setup_tracing(settings: &Settings) -> tracing_appender::non_blocking::WorkerGuard {
    let (non_blocking, guard) = tracing_appender::non_blocking(std::io::stdout());
    let filter = tracing_subscriber::filter::Targets::new()
        .with_target(env!("CARGO_BIN_NAME"), settings.log.level)
        .with_target("gateway_rs", settings.log.level)
        .with_default(Level::INFO);

    let stdout_log = tracing_subscriber::fmt::layer()
        .compact()
        .with_timer(settings.log.time_formatter())
        .with_writer(non_blocking);

    tracing_subscriber::registry()
        .with(stdout_log)
        .with(filter)
        .init();
    guard
}

pub fn main() -> Result {
    let cli = Cli::parse();

    let settings = Settings::new(&cli.config)?;

    // This `main()` returns a result only for errors we can't easily
    // intercept and log. An example is config file parsing. The
    // config file is the source of truth for the kind of logger we
    // need to build. Any `Err's returned are displayed to STDERR and
    // result in a non-zero exit code. And due to the behavior of our
    // logger, simply calling `exit()` early prevents any error
    // logging from reaching its destination.
    let retcode = {
        let _guard = setup_tracing(&settings);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime build");

        // Start the runtime
        let res = runtime.block_on(async {
            let (shutdown_trigger, shutdown_listener) = triggered::trigger();
            tokio::spawn(async move {
                let mut in_buf = [0u8; 64];
                let mut stdin = tokio::io::stdin();
                loop {
                    tokio::select!(
                        _ = signal::ctrl_c() => break,
                        read = stdin.read(&mut in_buf), if cli.stdin => if let Ok(0) = read { break },
                    )
                }
                shutdown_trigger.trigger()
            });
            run(cli, settings, &shutdown_listener).await
        });
        runtime.shutdown_timeout(Duration::from_secs(0));

        match res {
            Err(e) => {
                error!("{e}");
                1
            }
            _ => 0,
        }
    };

    std::process::exit(retcode);
}

pub async fn run(cli: Cli, settings: Settings, shutdown_listener: &triggered::Listener) -> Result {
    debug!(settings = %cli.config.display(), "starting");
    match cli.cmd {
        Cmd::Key(cmd) => cmd.run(settings).await,
        Cmd::Info(cmd) => cmd.run(settings).await,
        Cmd::Add(cmd) => cmd.run(settings).await,
        Cmd::Server(cmd) => cmd.run(shutdown_listener, settings).await,
    }
}
