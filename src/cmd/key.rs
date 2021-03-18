use crate::*;
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
        println!("{}", settings.keypair.public_key.to_string());
        Ok(())
    }
}
