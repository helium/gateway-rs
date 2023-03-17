use crate::{
    api::LocalClient,
    cmd::*,
    packet_router::RouterStatus,
    settings::{self, Settings},
    Region, Result,
};
use angry_purple_tiger::AnimalName;
use helium_crypto::PublicKey;

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
        let mut info_cache = InfoCache::new(settings.api);
        let mut info: HashMap<String, serde_json::Value> = HashMap::new();
        for key in &self.keys {
            info.insert(key.to_string(), key.to_status(&mut info_cache).await?);
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
struct InfoCache {
    port: u16,
    public_keys: Option<(PublicKey, PublicKey)>,
    region: Option<Region>,
    router: Option<RouterStatus>,
}

impl InfoCache {
    fn new(port: u16) -> Self {
        Self {
            port,
            public_keys: None,
            region: None,
            router: None,
        }
    }

    async fn _public_keys(&mut self) -> Result<(PublicKey, PublicKey)> {
        if let Some(public_keys) = &self.public_keys {
            return Ok(public_keys.clone());
        }
        let mut client = LocalClient::new(self.port).await?;
        let public_keys = client.pubkey().await?;
        self.public_keys = Some(public_keys.clone());
        Ok(public_keys)
    }

    async fn public_key(&mut self) -> Result<PublicKey> {
        let (public_key, _) = self._public_keys().await?;
        Ok(public_key)
    }

    async fn onboarding_key(&mut self) -> Result<PublicKey> {
        let (_, onboarding_key) = self._public_keys().await?;
        Ok(onboarding_key)
    }

    async fn region(&mut self) -> Result<Region> {
        if let Some(region) = self.region {
            return Ok(region);
        }
        let mut client = LocalClient::new(self.port).await?;
        let region = client.region().await?;
        self.region = Some(region);
        Ok(region)
    }

    pub async fn router(&mut self) -> Result<RouterStatus> {
        if let Some(router) = &self.router {
            return Ok(router.clone());
        }
        let mut client = LocalClient::new(self.port).await?;
        let router = client.router().await?;
        self.router = Some(router.clone());
        Ok(router)
    }
}

impl InfoKey {
    async fn to_status(self, cache: &mut InfoCache) -> Result<serde_json::Value> {
        let v = match self {
            Self::Fw => {
                let version = settings::version();
                json!(format!("{version}"))
            }
            Self::Key => {
                json!(cache.public_key().await?.to_string())
            }
            Self::Onboarding => {
                json!(cache.onboarding_key().await?.to_string())
            }
            Self::Name => {
                let public_key = cache.public_key().await?.to_string();
                let name = public_key.parse::<AnimalName>().unwrap().to_string();
                json!(name)
            }
            Self::Region => {
                json!(cache.region().await?.to_string())
            }
            Self::Router => {
                json!(cache.router().await?)
            }
        };
        Ok(v)
    }
}
