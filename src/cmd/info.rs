use crate::{
    api::{HeightRes, LocalClient},
    cmd::*,
    keyed_uri::KeyedUri,
    settings::{self, Settings},
    Error, Region, Result,
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
    Name,
    Gateway,
    Region,
}

#[derive(Debug, Clone)]
pub struct InfoKeys(pub(crate) Vec<InfoKey>);

/// Info command. Retrieve all or a subset of information from the running
/// service.
#[derive(Debug, StructOpt)]
pub struct Cmd {
    /// Information keys to fetch
    #[structopt(long, multiple = false, default_value = "fw,key,name,gateway")]
    pub keys: InfoKeys,
}

impl Cmd {
    pub async fn run(&self, settings: Settings) -> Result {
        let mut info_cache = InfoCache::new(settings.update.platform.clone(), settings.api);
        let mut info: HashMap<String, serde_json::Value> = HashMap::new();
        for key in &self.keys.0 {
            info.insert(key.to_string(), key.to_status(&mut info_cache).await?);
        }
        print_json(&info)
    }
}

const INFO_FW: &str = "fw";
const INFO_KEY: &str = "key";
const INFO_NAME: &str = "name";
const INFO_GATEWAY: &str = "gateway";
const INFO_REGION: &str = "region";

impl fmt::Display for InfoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Fw => INFO_FW,
            Self::Key => INFO_KEY,
            Self::Name => INFO_NAME,
            Self::Gateway => INFO_GATEWAY,
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
            INFO_NAME => Ok(Self::Name),
            INFO_GATEWAY => Ok(Self::Gateway),
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
    platform: String,
    port: u16,
    public_key: Option<PublicKey>,
    height: Option<HeightRes>,
    region: Option<Region>,
}

impl InfoCache {
    fn new(platform: String, port: u16) -> Self {
        Self {
            platform,
            port,
            public_key: None,
            height: None,
            region: None,
        }
    }

    async fn public_key(&mut self) -> Result<PublicKey> {
        if let Some(public_key) = &self.public_key {
            return Ok(public_key.clone());
        }
        let mut client = LocalClient::new(self.port).await?;
        let public_key = client.pubkey().await?;
        self.public_key = Some(public_key.clone());
        Ok(public_key)
    }

    async fn _height(&mut self) -> Result<HeightRes> {
        if let Some(height) = &self.height {
            return Ok(height.clone());
        }
        let mut client = LocalClient::new(self.port).await?;
        let height = client.height().await?;
        self.height = Some(height.clone());
        Ok(height)
    }

    async fn height(&mut self) -> Result<u64> {
        let height = self._height().await?;
        Ok(height.height)
    }

    async fn block_age(&mut self) -> Result<u64> {
        let height = self._height().await?;
        Ok(height.block_age)
    }

    async fn gateway(&mut self) -> Result<crate::KeyedUri> {
        let height = self._height().await?;
        height
            .gateway
            .ok_or_else(|| Error::custom("No uri for gateway"))
            .and_then(KeyedUri::try_from)
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
                let platform = &cache.platform;
                let version = settings::version();
                json!(format!("{platform}-{version}"))
            }
            Self::Key => {
                json!(cache.public_key().await?.to_string())
            }
            Self::Name => {
                let public_key = cache.public_key().await?.to_string();
                let name = public_key.parse::<AnimalName>().unwrap().to_string();
                json!(name)
            }
            Self::Gateway => {
                let gateway = cache.gateway().await?;
                json!({
                    "uri" : gateway.uri.to_string(),
                    "key" : gateway.pubkey.to_string(),
                    "height": cache.height().await?,
                    "block_age": cache.block_age().await?,
                })
            }
            Self::Region => {
                json!(cache.region().await?.to_string())
            }
        };
        Ok(v)
    }
}
