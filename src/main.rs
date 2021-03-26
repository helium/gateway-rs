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
    /// Configuration folder to use. default.toml will be loaded first and any
    /// custom settings in settings.toml merged in.
    #[structopt(short = "c", default_value = "/etc/helium_gateway")]
    config: PathBuf,

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
            let decorator = slog_term::PlainDecorator::new(std::io::stdout());
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
            .start()
            .expect("daemon start");
    }

    let settings = Settings::new(&cli.config)?;
    let logger = mk_logger(&settings);
    let scope_guard = slog_scope::set_global_logger(logger);
    let run_logger = slog_scope::logger().new(o!());
    // Start the runtime after the daemon fork
    let res = tokio::runtime::Builder::new_current_thread()
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
