use crate::{api::GatewayClient, cmd::*, Settings};
use angry_purple_tiger::AnimalName;
use serde_json::json;
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
    pub async fn run(&self, _settings: Settings) -> Result {
        let mut client = GatewayClient::new().await?;
        let public_key = client.pubkey().await?;
        let key = public_key.to_string();
        let table = json!({
            "address": key,
            "name": key.parse::<AnimalName>().unwrap().to_string(),
        });
        print_json(&table)
    }
}
