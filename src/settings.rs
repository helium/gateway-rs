use crate::*;
use config::{Config, Environment, File};
use helium_crypto::{ecc_compact, Network};
use helium_proto::Region;
use http::uri::Uri;
use rand::rngs::OsRng;
use serde::{de, Deserialize, Deserializer};
use std::{collections::HashMap, net::SocketAddr, path::Path, sync::Arc};

pub fn version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("unable to parse version")
}

/// A URI that has an associated public key
#[derive(Debug, Clone, Deserialize)]
pub struct KeyedUri {
    #[serde(deserialize_with = "deserialize_uri")]
    pub uri: Uri,
    #[serde(deserialize_with = "deserialize_pubkey")]
    pub public_key: PublicKey,
}

/// Settings are all the configuration parameters the service needs to operate.
#[derive(Debug, Deserialize)]
pub struct Settings {
    /// The listen address to use for listening for the semtech UDP packet forwarder.
    /// Default "127.0.0.1:1680"
    #[serde(deserialize_with = "deserialize_listen_addr")]
    pub listen_addr: SocketAddr,
    /// The location of the keypair binary file for the gateway. Defaults to
    /// "/etc/helium_gateway/keypair.bin". If the keyfile is not found there a new
    /// one is generated and saved in that location.
    #[serde(deserialize_with = "deserialize_keypair")]
    pub keypair: Arc<Keypair>,
    /// The lorawan region to use. This value should line up with the configured
    /// region of the semtech packet forwarder. Defaults to "US91%"
    #[serde(deserialize_with = "deserialize_region")]
    pub region: Region,
    /// Log settings
    pub log: LogSettings,
    /// Update settings
    pub update: UpdateSettings,
    /// The router to deliver packets to when no routers are found while
    /// processing a packet.
    pub router: HashMap<String, KeyedUri>,
    /// The validator(s) to query for chain related state. Defaults to a Helium
    /// validator.
    pub gateways: Vec<KeyedUri>,
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
    pub level: slog::Level,

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
    /// Which udpate channel to use (alpha, beta, release, semver).
    /// Defaults to semver which is the channel specified in the running app.
    #[serde(deserialize_with = "deserialize_update_channel")]
    pub channel: releases::Channel,
    /// The platform identifier to use for released packages (default: klkgw)
    pub platform: String,
    /// The github release url to use (default
    /// https://api.github.com/repos/helium/gateway-rs/releases)
    #[serde(deserialize_with = "deserialize_uri")]
    pub uri: Uri,
    /// The command to use to install an update. There will be just one
    /// parameter which is the path to the new package to install.
    pub command: String,
}

impl Settings {
    /// Load Settings from a given path. Settings are loaded from a default.toml
    /// file in the given path, followed by merging in an optional settings.toml
    /// in the same folder.
    ///
    /// Environemnt overrides have the same name as the entries in the settings
    /// file in uppercase and prefixed with "GW_". For example "GW_KEY" will
    /// override the key file location.
    pub fn new(path: &Path) -> Result<Self> {
        let mut c = Config::new();
        let default_file = path.join("default.toml");
        // Load default config and merge in overrides
        c.merge(File::with_name(default_file.to_str().expect("file name")))?;
        let settings_file = path.join("settings.toml");
        if settings_file.exists() {
            c.merge(File::with_name(settings_file.to_str().expect("file name")))?;
        }
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `GW_DEBUG=1 ./target/app` would set the `debug` key
        c.merge(Environment::with_prefix("gw"))?;
        c.try_into().map_err(|e| e.into())
    }

    pub fn default_router(&self) -> &KeyedUri {
        &self.router[&self.update.channel.to_string()]
    }
}

fn deserialize_keypair<'de, D>(d: D) -> std::result::Result<Arc<Keypair>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    match keypair::load_from_file(&s) {
        Ok(k) => Ok(Arc::new(k)),
        Err(Error::IO(io_error)) if io_error.kind() == std::io::ErrorKind::NotFound => {
            let new_key = ecc_compact::Keypair::generate(Network::MainNet, &mut OsRng);
            keypair::save_to_file(&new_key, &s).map_err(|e| {
                de::Error::custom(format!("unable to save key file \"{}\": {:?}", s, e))
            })?;
            Ok(Arc::new(new_key))
        }
        Err(err) => Err(de::Error::custom(format!(
            "unable to load key file \"{}\": {:?}",
            s, err
        ))),
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
        "AS923_1" => Region::As9231,
        "AS923_2" => Region::As9232,
        "AS923_3" => Region::As9233,
        "AS923_4" => Region::As9234,
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

fn deserialize_log_level<'de, D>(d: D) -> std::result::Result<slog::Level, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    s.parse()
        .map_err(|_| de::Error::custom(format!("invalid log level \"{}\"", s)))
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
    let channel_str = String::deserialize(d)?.to_lowercase();
    match channel_str.parse::<releases::Channel>() {
        Ok(channel) => Ok(channel),
        Err(_) => {
            return Err(de::Error::custom(format!(
                "unsupported update channel: \"{}\"",
                channel_str
            )))
        }
    }
}

fn deserialize_uri<'de, D>(d: D) -> std::result::Result<Uri, D::Error>
where
    D: Deserializer<'de>,
{
    let uri_string = String::deserialize(d)?;
    match uri_string.parse() {
        Ok(uri) => Ok(uri),
        Err(err) => Err(de::Error::custom(format!("invalid uri: \"{}\"", err))),
    }
}

fn deserialize_pubkey<'de, D>(d: D) -> std::result::Result<PublicKey, D::Error>
where
    D: Deserializer<'de>,
{
    let key_string = String::deserialize(d)?;
    match key_string.parse() {
        Ok(key) => Ok(key),
        Err(err) => Err(de::Error::custom(format!(
            "invalid public key: \"{}\"",
            err
        ))),
    }
}
