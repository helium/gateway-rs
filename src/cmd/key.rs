use crate::{
    cmd::info::{self, InfoKey, InfoKeys},
    Result, Settings,
};
use structopt::StructOpt;

/// Commands on gateway keys
#[derive(Debug, StructOpt)]
pub enum Cmd {
    Info(Info),
}

/// Commands on gateway keys
#[derive(Debug, StructOpt)]
pub struct Info {}

impl Cmd {
    pub async fn run(&self, settings: Settings) -> Result {
        match self {
            Cmd::Info(cmd) => cmd.run(settings).await,
        }
    }
}

impl Info {
    pub async fn run(&self, settings: Settings) -> Result {
        let cmd = info::Cmd {
            keys: InfoKeys(vec![InfoKey::Name, InfoKey::Key, InfoKey::OnboardingKey]),
        };
        cmd.run(settings).await
    }
}
