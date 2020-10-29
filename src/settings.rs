use crate::{key, result::Result};
use config::{Config, Environment, File};
use helium_proto::Region;
use serde::{de, Deserialize, Deserializer};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

/// Settings are all the configuration parameters the service needs to operate.
#[derive(Debug, Deserialize)]
pub struct Settings {
    /// The listen address to use for listening for the semtech UDP packet forwarder.
    /// Default "127.0.0.1:1680"
    #[serde(deserialize_with = "deserialize_listen_addr")]
    pub listen_addr: SocketAddr,
    /// The location of the key pem file for the gateway. Defaults to
    /// "/etc/gateway/gateway_key.pem". If the keyfile is not found there a new
    /// one is generated and saved in that location.
    #[serde(deserialize_with = "deserialize_key")]
    pub key: Arc<key::Key>,
    /// The lorawan region to use. This value should line up with the configured
    /// region of the semtech packet forwarder. Defaults to "US91%"
    #[serde(deserialize_with = "deserialize_region")]
    pub region: Region,
}

impl Settings {
    /// Load Settings from a given path. Settings are loaded by constructing
    /// default settings, and then merging in a given path, followed by merging
    /// in any environment overrides.
    ///
    /// Environemnt overrides have the same name as the entries in the settings
    /// file in uppercase and prefixed with "GW_". For example "GW_KEY" will
    /// override the key file location.
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let mut c = Config::new();
        c.set_default("key", "/etc/gateway/gateway_key.pem")?;
        c.set_default("listen_addr", "127.0.0.1:1680")?;
        c.set_default("region", "US915")?;
        if let Some(p) = path {
            let path_str = p.to_str().unwrap();
            c.merge(File::with_name(&path_str))?;
        }
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `GW_DEBUG=1 ./target/app` would set the `debug` key
        c.merge(Environment::with_prefix("gw"))?;
        c.try_into().map_err(|e| e.into())
    }
}

fn deserialize_key<'de, D>(d: D) -> std::result::Result<Arc<key::Key>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    match key::Key::load(&s) {
        Ok(k) => Ok(Arc::new(k)),
        Err(_) => {
            let new_key = key::Key::generate().map_err(de::Error::custom)?;
            new_key.save(&s).map_err(|e| {
                de::Error::custom(format!("unable to save key file \"{}\": {:?}", s, e))
            })?;
            Ok(Arc::new(new_key))
        }
    }
}

fn deserialize_listen_addr<'de, D>(d: D) -> std::result::Result<SocketAddr, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    s.parse()
        .map_err(|e| de::Error::custom(format!("invalid listen address \"{}\": {}", s, e)))
}

fn deserialize_region<'de, D>(d: D) -> std::result::Result<Region, D::Error>
where
    D: Deserializer<'de>,
{
    let region = match String::deserialize(d)?.as_str() {
        "US915" => Region::Us915,
        "EU868" => Region::Eu868,
        "EU433" => Region::Eu433,
        "CN470" => Region::Cn470,
        "CN779" => Region::Cn779,
        "AU915" => Region::Au915,
        "AS923" => Region::As923,
        "KR920" => Region::Kr920,
        "IN865" => Region::In865,
        unsupported => {
            return Err(de::Error::custom(format!(
                "unsupported region: {}",
                unsupported
            )))
        }
    };
    Ok(region)
}
