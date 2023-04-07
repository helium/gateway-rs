use crate::{
    api::LocalClient,
    cmd::*,
    settings::{self, Settings},
    Result,
};
use angry_purple_tiger::AnimalName;

use serde_json::json;
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, clap::ValueEnum, PartialOrd, Ord, Copy, PartialEq, Eq)]
pub enum InfoKey {
    Fw,
    Key,
    Onboarding,
    Name,
    Region,
    Router,
}

/// Info command. Retrieve all or a subset of information from the running
/// service.
#[derive(Debug, clap::Args)]
pub struct Cmd {
    /// Information keys to fetch
    #[arg(value_enum, required = true)]
    pub keys: Vec<InfoKey>,
}

impl Cmd {
    pub async fn run(&self, settings: Settings) -> Result {
        let mut client = LocalClient::new(&settings.api).await?;
        let mut info: HashMap<String, serde_json::Value> = HashMap::new();
        for key in &self.keys {
            info.insert(key.to_string(), key.to_status(&mut client).await?);
        }
        print_json(&info)
    }
}

impl fmt::Display for InfoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Fw => "fw",
            Self::Key => "key",
            Self::Onboarding => "onboarding",
            Self::Name => "name",
            Self::Region => "region",
            Self::Router => "router",
        };
        f.write_str(s)
    }
}

impl InfoKey {
    async fn to_status(self, client: &mut LocalClient) -> Result<serde_json::Value> {
        let (public_key, _) = client.pubkey().await?;
        let v = match self {
            Self::Fw => {
                let version = settings::version();
                json!(format!("{version}"))
            }
            Self::Key => {
                json!(public_key)
            }
            Self::Onboarding => {
                let (_, onboarding_key) = client.pubkey().await?;
                json!(onboarding_key)
            }
            Self::Name => {
                let name = public_key
                    .to_string()
                    .parse::<AnimalName>()
                    .unwrap()
                    .to_string();
                json!(name)
            }
            Self::Region => {
                let region = client.region().await?;
                let maybe_region = if region.is_unknown() {
                    None
                } else {
                    Some(region)
                };
                json!(maybe_region.map(|region| region.to_string()))
            }
            Self::Router => {
                let router = client.router().await?;
                json!(router)
            }
        };
        Ok(v)
    }
}
