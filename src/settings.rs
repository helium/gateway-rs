use crate::{key, result::Result};
use config::{Config, Environment, File};
use helium_proto::Region;
use reqwest::{Certificate, Url};
use serde::{de, Deserialize, Deserializer};
use std::{net::SocketAddr, path::PathBuf};

/// The Helium staging router URL. Used as one of the default routers.
pub const HELIUM_STAGING_ROUTER: &str = "http://54.176.88.149:20443";

/// Settings are all the configuration parameters the service needs to operate.
#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    /// The listen address to use for listening for the semtech UDP packet forwarder.
    /// Default "127.0.0.1:1680"
    #[serde(deserialize_with = "deserialize_listen_addr")]
    pub listen_addr: SocketAddr,
    /// The location of the key pem file for the gateway. Defaults to
    /// "/etc/gateway/gateway_key.pem". If the keyfile is not found there a new
    /// one is generated and saved in that location.
    #[serde(deserialize_with = "deserialize_key")]
    pub key: key::Key,
    /// The lorawan region to use. This value should line up with the configured
    /// region of the semtech packet forwarder. Defaults to "US91%"
    #[serde(deserialize_with = "deserialize_region")]
    pub region: Region,
    /// The root certificates to use when connecting to remote hosts
    #[serde(deserialize_with = "deserialize_root_certs")]
    pub root_certs: Vec<Certificate>,
    /// The router(s) to deliver packets to. Defaults to the Helium staging and
    /// production routers.
    #[serde(deserialize_with = "deserialize_routers")]
    pub routers: Vec<Url>,
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
        c.set_default("root_certs", Vec::<String>::new())?;
        c.set_default("routers", vec![HELIUM_STAGING_ROUTER])?;
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

fn deserialize_key<'de, D>(d: D) -> std::result::Result<key::Key, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    match key::Key::load(&s) {
        Ok(k) => Ok(k),
        Err(_) => {
            let new_key = key::Key::generate().map_err(de::Error::custom)?;
            new_key.save(&s).map_err(|e| {
                de::Error::custom(format!("unable to save key file \"{}\": {:?}", s, e))
            })?;
            Ok(new_key)
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

fn deserialize_root_certs<'de, D>(d: D) -> std::result::Result<Vec<Certificate>, D::Error>
where
    D: Deserializer<'de>,
{
    let cert_files = Vec::<String>::deserialize(d)?;
    let mut result = Vec::with_capacity(cert_files.len());
    for cert_file in cert_files {
        let cert = Certificate::from_pem(cert_file.as_bytes())
            .map_err(|e| de::Error::custom(format!("Could not load pem: {}: {}", cert_file, e)))?;
        result.push(cert);
    }
    Ok(result)
}

fn deserialize_routers<'de, D>(d: D) -> std::result::Result<Vec<Url>, D::Error>
where
    D: Deserializer<'de>,
{
    let entries = Vec::<String>::deserialize(d)?;
    let mut result = Vec::with_capacity(entries.len());
    for entry in entries {
        let router = Url::parse(&entry)
            .map_err(|e| de::Error::custom(format!("Could not parse router url: {}", e)))?;
        result.push(router);
    }
    Ok(result)
}
