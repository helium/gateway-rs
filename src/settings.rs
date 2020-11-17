use crate::{
    key,
    result::Result,
    router::{Certificate, Url},
    updater,
};
use config::{Config, Environment, File};
use helium_proto::Region;
use serde::{de, Deserialize, Deserializer};
use std::{net::SocketAddr, path::PathBuf};

/// The Helium staging router URL. Used as one of the default routers.
pub const HELIUM_STAGING_ROUTER: &str = "http://54.176.88.149:20443/v1/router/message";
pub const GITHUB_RELEASES: &str = "https://api.github.com/repos/helium/gateway-rs/releases";

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
    /// Log settings
    pub log: LogSettings,
    /// Update settings
    pub update: UpdateSettings,
}

/// The method to use for logging.
#[derive(Debug, Deserialize, Clone)]
pub enum LogMethod {
    /// Display logging information on stdout
    Stdio,
    /// Send logging information to syslog
    Syslog,
}

/// Settings for log method and level to be used by the running service.
#[derive(Debug, Deserialize, Clone)]
pub struct LogSettings {
    /// Log level to show (default info)
    #[serde(deserialize_with = "deserialize_log_level")]
    pub level: log::LevelFilter,

    ///  Which log method to use (stdio or syslog, default stdio)
    #[serde(deserialize_with = "deserialize_log_method")]
    pub method: LogMethod,

    /// Whehter to show timestamps in the stdio output stream (default false)
    pub timestamp: bool,
}

/// Settings for log method and level to be used by the running service.
#[derive(Debug, Deserialize, Clone)]
pub struct UpdateSettings {
    /// Whether the auto-update system is enabled (default: true)
    pub enabled: bool,
    /// How often to check for updates (in minutes, default: 10)
    pub interval: u32,
    ///  Which udpate channel to use (alpha, beta, release, default: release)
    #[serde(deserialize_with = "deserialize_update_channel")]
    pub channel: updater::Channel,
    /// The platform identifier to use for released packages (default: keros)
    pub platform: String,
    /// The github release url to use (default
    /// https://api.github.com/repos/helium/gateway-rs/releases)
    #[serde(deserialize_with = "deserialize_url")]
    pub url: Url,
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
        c.set_default("key", "/etc/helium_gateway/key.pem")?;
        c.set_default("listen_addr", "127.0.0.1:1680")?;
        c.set_default("region", "US915")?;
        c.set_default("root_certs", Vec::<String>::new())?;
        c.set_default("routers", vec![HELIUM_STAGING_ROUTER])?;
        c.set_default("log.level", "info")?;
        c.set_default("log.method", "stdio")?;
        c.set_default("log.timestamp", "false")?;
        c.set_default("update.enabled", "true")?;
        c.set_default("update.channel", "release")?;
        c.set_default("update.platform", "keros")?;
        c.set_default("update.interval", 10)?;
        c.set_default("update.url", GITHUB_RELEASES)?;
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

fn deserialize_log_level<'de, D>(d: D) -> std::result::Result<log::LevelFilter, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    s.parse()
        .map_err(|e| de::Error::custom(format!("invalid log level \"{}\": {}", s, e)))
}

fn deserialize_log_method<'de, D>(d: D) -> std::result::Result<LogMethod, D::Error>
where
    D: Deserializer<'de>,
{
    let method = match String::deserialize(d)?.to_lowercase().as_str() {
        "stdio" => LogMethod::Stdio,
        "syslog" => LogMethod::Syslog,
        unsupported => {
            return Err(de::Error::custom(format!(
                "unsupported log method: \"{}\"",
                unsupported
            )))
        }
    };
    Ok(method)
}

fn deserialize_update_channel<'de, D>(d: D) -> std::result::Result<updater::Channel, D::Error>
where
    D: Deserializer<'de>,
{
    let channel = match String::deserialize(d)?.to_lowercase().as_str() {
        "alpha" => updater::Channel::Alpha,
        "beta" => updater::Channel::Beta,
        "release" | "" => updater::Channel::Release,
        unsupported => {
            return Err(de::Error::custom(format!(
                "unsupported update channel: \"{}\"",
                unsupported
            )))
        }
    };
    Ok(channel)
}

fn deserialize_url<'de, D>(d: D) -> std::result::Result<reqwest::Url, D::Error>
where
    D: Deserializer<'de>,
{
    let url_string = String::deserialize(d)?;
    match reqwest::Url::parse(&url_string) {
        Ok(url) => Ok(url),
        Err(err) => Err(de::Error::custom(format!(
            "invalid url format: \"{}\"",
            err
        ))),
    }
}
