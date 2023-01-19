use crate::{
    cmd::info::{self, InfoKey},
    Result, Settings,
};

/// Commands on gateway keys
#[derive(Debug, clap::Args)]
pub struct Cmd {
    #[command(subcommand)]
    command: KeyCmd,
}

#[derive(Debug, clap::Subcommand)]
pub enum KeyCmd {
    Info(Info),
}

/// Commands on gateway keys
#[derive(Debug, clap::Args)]
pub struct Info {}

impl Cmd {
    pub async fn run(&self, settings: Settings) -> Result {
        self.command.run(settings).await
    }
}

impl KeyCmd {
    pub async fn run(&self, settings: Settings) -> Result {
        match self {
            Self::Info(cmd) => cmd.run(settings).await,
        }
    }
}

impl Info {
    pub async fn run(&self, settings: Settings) -> Result {
        let cmd = info::Cmd {
            keys: vec![InfoKey::Name, InfoKey::Key, InfoKey::Onboarding],
        };
        cmd.run(settings).await
    }
}
