use crate::{
    api::LocalClient,
    cmd::*,
    settings::{self, Settings},
    Region, Result,
};
use angry_purple_tiger::AnimalName;
use helium_crypto::PublicKey;

use serde_json::json;
use std::collections::HashMap;
use std::{fmt, str::FromStr};
use structopt::StructOpt;

#[derive(Debug, Clone)]
pub enum InfoKey {
    Fw,
    Key,
    OnboardingKey,
    Name,
    Region,
}

#[derive(Debug, Clone)]
pub struct InfoKeys(pub(crate) Vec<InfoKey>);

/// Info command. Retrieve all or a subset of information from the running
/// service.
#[derive(Debug, StructOpt)]
pub struct Cmd {
    /// Information keys to fetch
    #[structopt(
        long,
        short,
        multiple = false,
        default_value = "fw,key,onboarding,name,region,gateway"
    )]
    pub keys: InfoKeys,
}

impl Cmd {
    pub async fn run(&self, settings: Settings) -> Result {
        let mut info_cache = InfoCache::new(settings.api);
        let mut info: HashMap<String, serde_json::Value> = HashMap::new();
        for key in &self.keys.0 {
            info.insert(key.to_string(), key.to_status(&mut info_cache).await?);
        }
        print_json(&info)
    }
}

const INFO_FW: &str = "fw";
const INFO_KEY: &str = "key";
const INFO_ONBOARDING_KEY: &str = "onboarding";
const INFO_NAME: &str = "name";
const INFO_REGION: &str = "region";

impl fmt::Display for InfoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Fw => INFO_FW,
            Self::Key => INFO_KEY,
            Self::OnboardingKey => INFO_ONBOARDING_KEY,
            Self::Name => INFO_NAME,
            Self::Region => INFO_REGION,
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoKeyParseError(String);

impl fmt::Display for InfoKeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid key: {}", self.0)
    }
}

impl FromStr for InfoKey {
    type Err = InfoKeyParseError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            INFO_FW => Ok(Self::Fw),
            INFO_KEY => Ok(Self::Key),
            INFO_ONBOARDING_KEY => Ok(Self::OnboardingKey),
            INFO_NAME => Ok(Self::Name),
            INFO_REGION => Ok(Self::Region),
            invalid => Err(InfoKeyParseError(invalid.to_string())),
        }
    }
}

impl std::str::FromStr for InfoKeys {
    type Err = InfoKeyParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let keys: std::result::Result<Vec<InfoKey>, _> = s
            .split(',')
            .map(|x| x.trim().to_owned().parse::<InfoKey>())
            .collect();

        Ok(InfoKeys(keys?))
    }
}

struct InfoCache {
    port: u16,
    public_keys: Option<(PublicKey, PublicKey)>,
    region: Option<Region>,
}

impl InfoCache {
    fn new(port: u16) -> Self {
        Self {
            port,
            public_keys: None,
            region: None,
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
}

impl InfoKey {
    async fn to_status(&self, cache: &mut InfoCache) -> Result<serde_json::Value> {
        let v = match self {
            Self::Fw => {
                let version = settings::version();
                json!(format!("{version}"))
            }
            Self::Key => {
                json!(cache.public_key().await?.to_string())
            }
            Self::OnboardingKey => {
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
        };
        Ok(v)
    }
}
