use env_logger::Env;
use gateway_rs::{cmd, result::Result, settings::Settings};
use std::path::PathBuf;
use structopt::StructOpt;
use syslog::{BasicLogger, Facility, Formatter3164};

#[derive(Debug, StructOpt)]
#[structopt(name = env!("CARGO_BIN_NAME"), version = env!("CARGO_PKG_VERSION"), about = "Helium Light Gateway")]
pub struct Cli {
    /// Config file to load. Defaults to "config/default"
    #[structopt(short = "c")]
    config: Option<PathBuf>,

    /// Use syslog for logging. If not specified stdout is used.
    #[structopt(long)]
    use_syslog: bool,

    /// Daemonize the application
    #[structopt(long)]
    daemon: bool,

    #[structopt(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, StructOpt)]
pub enum Cmd {
    Key(cmd::key::Cmd),
    Server(cmd::server::Cmd),
}

fn install_logger(use_syslog: bool) {
    if use_syslog {
        use log::LevelFilter;
        let formatter = Formatter3164 {
            facility: Facility::LOG_USER,
            hostname: None,
            process: env!("CARGO_BIN_NAME").into(),
            pid: std::process::id() as i32,
        };
        let logger = syslog::unix(formatter).expect("could not connect to syslog");
        log::set_boxed_logger(Box::new(BasicLogger::new(logger)))
            .map(|()| log::set_max_level(LevelFilter::Info))
            .expect("coult not set log level");
    } else {
        env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    }
}

pub fn main() -> Result {
    let cli = Cli::from_args();
    if cli.daemon {
        daemonize::Daemonize::new()
            .pid_file(format!("/var/run/{}.pid", env!("CARGO_BIN_NAME")))
            .start()?;
    }

    let settings = Settings::new(cli.config.clone())?;
    install_logger(cli.use_syslog);
    // Start the runtime after the daemon fork
    tokio::runtime::Builder::new()
        .threaded_scheduler()
        .enable_all()
        .build()?
        .block_on(async {
            let (shutdown_trigger, shutdown_listener) = triggered::trigger();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                shutdown_trigger.trigger();
            });
            run(cli, settings, &shutdown_listener).await
        })
}

pub async fn run(cli: Cli, settings: Settings, shutdown_listener: &triggered::Listener) -> Result {
    match cli.cmd {
        Cmd::Key(cmd) => cmd.run(settings).await,
        Cmd::Server(cmd) => cmd.run(shutdown_listener, settings).await,
    }
}
