use gateway_rs::{
    cmd,
    error::Result,
    settings::{LogMethod, Settings},
};
use slog::{self, o, Drain, Logger};
use std::path::PathBuf;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = env!("CARGO_BIN_NAME"), version = env!("CARGO_PKG_VERSION"), about = "Helium Light Gateway")]
pub struct Cli {
    /// Config file to load. Defaults to "config/default"
    #[structopt(short = "c")]
    config: Option<PathBuf>,

    /// Daemonize the application
    #[structopt(long)]
    daemon: bool,

    #[structopt(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, StructOpt)]
pub enum Cmd {
    Key(cmd::key::Cmd),
    Update(cmd::update::Cmd),
    Server(cmd::server::Cmd),
}

fn mk_logger(settings: &Settings) -> Logger {
    let async_drain = match settings.log.method {
        LogMethod::Syslog => {
            let drain = slog_syslog::unix_3164(slog_syslog::Facility::LOG_USER)
                .unwrap()
                .fuse();
            slog_async::Async::new(drain)
                .build()
                .filter_level(settings.log.level)
                .fuse()
        }
        LogMethod::Stdio => {
            let decorator = slog_term::TermDecorator::new().build();
            let drain = slog_term::FullFormat::new(decorator).build().fuse();
            slog_async::Async::new(drain)
                .build()
                .filter_level(settings.log.level)
                .fuse()
        }
    };
    slog::Logger::root(async_drain, o!())
}

pub fn main() -> Result {
    let cli = Cli::from_args();
    if cli.daemon {
        daemonize::Daemonize::new()
            .pid_file(format!("/var/run/{}.pid", env!("CARGO_BIN_NAME")))
            .start()?;
    }

    let settings = Settings::new(cli.config.clone())?;
    let logger = mk_logger(&settings);
    let run_logger = logger.clone();
    let scope_guard = slog_scope::set_global_logger(logger);
    //    let _log_guard = slog_stdlog::init().unwrap();
    // Start the runtime after the daemon fork
    let res = tokio::runtime::Builder::new()
        .threaded_scheduler()
        .enable_all()
        .build()?
        .block_on(async {
            let (shutdown_trigger, shutdown_listener) = triggered::trigger();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                shutdown_trigger.trigger();
            });
            run(cli, settings, &shutdown_listener, run_logger).await
        });
    drop(scope_guard);
    res
}

pub async fn run(
    cli: Cli,
    settings: Settings,
    shutdown_listener: &triggered::Listener,
    logger: Logger,
) -> Result {
    match cli.cmd {
        Cmd::Key(cmd) => cmd.run(settings).await,
        Cmd::Update(cmd) => cmd.run(settings).await,
        Cmd::Server(cmd) => cmd.run(shutdown_listener, settings, &logger).await,
    }
}
