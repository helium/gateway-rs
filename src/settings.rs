use crate::{error::Result, keypair, releases, router::Url};
use config::{Config, Environment, File};
use helium_proto::Region;
use serde::{de, Deserialize, Deserializer};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

pub fn version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("unable to parse version")
}

/// The Helium staging router URL. Used as one of the default routers.
pub const HELIUM_STAGING_ROUTER: &str = "http://54.176.88.149:8080/v1/router/message";
pub const HELIUM_PRODUCTION_ROUTER: &str = "http://52.8.80.146:8080/v1/router/message";
pub const GITHUB_RELEASES: &str = "https://api.github.com/repos/helium/gateway-rs/releases";

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
    #[serde(deserialize_with = "deserialize_keypair")]
    pub keypair: Arc<keypair::Keypair>,
    /// The lorawan region to use. This value should line up with the configured
    /// region of the semtech packet forwarder. Defaults to "US91%"
    #[serde(deserialize_with = "deserialize_region")]
    pub region: Region,
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
#[derive(Debug, Deserialize)]
pub enum LogMethod {
    /// Display logging information on stdout
    Stdio,
    /// Send logging information to syslog
    Syslog,
}

/// Settings for log method and level to be used by the running service.
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
pub struct UpdateSettings {
    /// Whether the auto-update system is enabled (default: true)
    pub enabled: bool,
    /// How often to check for updates (in minutes, default: 10)
    pub interval: u32,
    ///  Which udpate channel to use (alpha, beta, release, default: the channel specified in the running app)
    #[serde(deserialize_with = "deserialize_update_channel")]
    pub channel: releases::Channel,
    /// The platform identifier to use for released packages (default: klkgw)
    pub platform: String,
    /// The github release url to use (default
    /// https://api.github.com/repos/helium/gateway-rs/releases)
    #[serde(deserialize_with = "deserialize_url")]
    pub url: Url,
    /// The command to use to install an update. There will be just one
    /// parameter which is the path to the new package to install.
    pub command: String,
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
        c.set_default("keypair", "/etc/helium_gateway/keypair.bin")?;
        c.set_default("listen_addr", "127.0.0.1:1680")?;
        c.set_default("region", "US915")?;
        c.set_default("root_certs", Vec::<String>::new())?;
        c.set_default(
            "routers",
            vec![HELIUM_STAGING_ROUTER, HELIUM_PRODUCTION_ROUTER],
        )?;
        c.set_default("log.level", "info")?;
        c.set_default("log.method", "stdio")?;
        c.set_default("log.timestamp", "false")?;
        c.set_default("update.enabled", "true")?;
        c.set_default(
            "update.channel",
            releases::Channel::from_version(&version()).to_string(),
        )?;
        c.set_default("update.interval", 10)?;
        c.set_default("update.url", GITHUB_RELEASES)?;
        c.set_default("update.platform", "klkgw")?;
        c.set_default("update.command", "/etc/helium_gateway/install_update")?;
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

fn deserialize_keypair<'de, D>(d: D) -> std::result::Result<Arc<keypair::Keypair>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    match keypair::Keypair::load(&s) {
        Ok(k) => Ok(Arc::new(k)),
        Err(_) => {
            let new_key = keypair::Keypair::generate().map_err(de::Error::custom)?;
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

fn deserialize_update_channel<'de, D>(d: D) -> std::result::Result<releases::Channel, D::Error>
where
    D: Deserializer<'de>,
{
    let channel = match String::deserialize(d)?.to_lowercase().as_str() {
        "alpha" => releases::Channel::Alpha,
        "beta" => releases::Channel::Beta,
        "release" | "" => releases::Channel::Release,
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
