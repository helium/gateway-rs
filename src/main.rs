use gateway_rs::{
    cmd,
    error::Result,
    settings::{LogMethod, Settings},
};
use slog::{self, debug, error, o, Drain, Logger};
use std::{io, path::PathBuf};
use structopt::StructOpt;
use tokio::{io::AsyncReadExt, signal, time::Duration};

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

    /// Monitor stdin and terminate when stdin closes.
    ///
    /// This flag is not cmopatible with the daemon flag
    #[structopt(long)]
    stdin: bool,

    #[structopt(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, StructOpt)]
pub enum Cmd {
    Key(cmd::key::Cmd),
    Info(cmd::info::Cmd),
    Update(cmd::update::Cmd),
    Server(cmd::server::Cmd),
    Add(Box<cmd::add::Cmd>),
}

/// An empty timestamp function for when timestamp should not be included in
/// the output. This is commonly used with logd on OpenWRT which adds its own
/// timestamp information after capturing stdout.
fn timestamp_none(_io: &mut dyn io::Write) -> io::Result<()> {
    Ok(())
}

fn mk_logger(settings: &Settings) -> Logger {
    let async_drain = match settings.log.method {
        LogMethod::Syslog => {
            let drain = slog_syslog::unix_3164(slog_syslog::Facility::LOG_USER)
                .unwrap()
                .fuse();
            slog_async::Async::new(drain)
                .build()
                .filter_level(settings.log.level.into())
                .fuse()
        }
        LogMethod::Stdio => {
            let decorator = slog_term::PlainDecorator::new(io::stdout());
            let timestamp = if settings.log.timestamp {
                slog_term::timestamp_local
            } else {
                timestamp_none
            };
            let drain = slog_term::FullFormat::new(decorator)
                .use_custom_timestamp(timestamp)
                .build()
                .fuse();
            slog_async::Async::new(drain)
                .build()
                .filter_level(settings.log.level.into())
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
    let _log_guard = slog_stdlog::init().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // Start the runtime after the daemon fork
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
        run(cli, settings, &shutdown_listener, run_logger.clone()).await
    });
    runtime.shutdown_timeout(Duration::from_secs(0));

    if let Err(e) = &res {
        error!(&run_logger, "{e}");
    };
    drop(scope_guard);
    Ok(())
}

pub async fn run(
    cli: Cli,
    settings: Settings,
    shutdown_listener: &triggered::Listener,
    logger: Logger,
) -> Result {
    debug!(logger, "starting"; "settings" => &cli.config.to_str());
    match cli.cmd {
        Cmd::Key(cmd) => cmd.run(settings).await,
        Cmd::Info(cmd) => cmd.run(settings).await,
        Cmd::Update(cmd) => cmd.run(settings).await,
        Cmd::Add(cmd) => cmd.run(settings).await,
        Cmd::Server(cmd) => cmd.run(shutdown_listener, settings, &logger).await,
    }
}
