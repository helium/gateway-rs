use crate::{
    api::GatewayStakingMode, releases, Error, KeyedUri, Keypair, PublicKey, Region, Result,
};
use config::{Config, Environment, File};
use http::uri::Uri;
pub use log_method::LogMethod;
use serde::Deserialize;
use std::{collections::HashMap, fmt, path::Path, str::FromStr, sync::Arc};

pub fn version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("unable to parse version")
}

/// Settings are all the configuration parameters the service needs to operate.
#[derive(Debug, Deserialize)]
pub struct Settings {
    /// The listen address to use for listening for the semtech UDP packet forwarder.
    /// Default "127.0.0.1:1680"
    #[serde(default = "default_listen")]
    pub listen: String,
    /// The listening network port for the grpc / jsonrpc API.
    /// Default 4467
    #[serde(default = "default_api")]
    pub api: u16,
    /// The location of the keypair binary file for the gateway. If the keyfile
    /// is not found there a new one is generated and saved in that location.
    pub keypair: Arc<Keypair>,
    /// The location of the onboarding keypair binary file for the gateway. If
    /// the keyfile is not found there a new one is generated and saved in that
    /// location.
    pub onboarding: Option<String>,
    /// The lorawan region to use. This value should line up with the configured
    /// region of the semtech packet forwarder. Defaults to "US915"
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
    /// Cache settings
    pub cache: CacheSettings,
}

/// Settings for log method and level to be used by the running service.
#[derive(Debug, Deserialize)]
pub struct LogSettings {
    /// Log level to show (default info)
    pub level: log_level::Level,

    ///  Which log method to use (stdio or syslog, default stdio)
    pub method: log_method::LogMethod,

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
    pub channel: releases::Channel,
    /// The platform identifier to use for released packages (default: klkgw)
    pub platform: String,
    /// The github release url to use (default
    /// https://api.github.com/repos/helium/gateway-rs/releases)
    #[serde(with = "http_serde::uri")]
    pub uri: Uri,
    /// The command to use to install an update. There will be just one
    /// parameter which is the path to the new package to install.
    pub command: String,
}

/// Settings for cache storage
#[derive(Debug, Deserialize, Clone)]
pub struct CacheSettings {
    // Maximum number of packets to queue up per router client
    pub max_packets: u16,
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
        let default_file = path.join("default.toml");
        let settings_file = path.join("settings.toml");
        Config::builder()
            // Source default config
            .add_source(File::with_name(default_file.to_str().expect("file name")))
            // Add optional settings file
            .add_source(File::with_name(settings_file.to_str().expect("file name")).required(false))
            // Add in settings from the environment (with a prefix of APP)
            // Eg.. `GW_DEBUG=1 ./target/app` would set the `debug` key
            .add_source(Environment::with_prefix("gw").separator("_"))
            .build()
            .and_then(|config| config.try_deserialize())
            .map_err(|e| e.into())
    }

    pub fn default_router(&self) -> Option<KeyedUri> {
        self.router
            .get(&self.update.channel.to_string())
            .map(|keyed_uri| keyed_uri.to_owned())
    }

    /// Returns the onboarding key for this gateway. The onboarding key is
    /// determined by the onboarding setting. If the onbaording setting is not
    /// present or there is any error retrievign the onboarding key from the
    /// confignred setting the public key of the gateawy is returned.
    pub fn onboarding_key(&self) -> PublicKey {
        self.onboarding.as_ref().map_or_else(
            || self.keypair.public_key().to_owned(),
            |str| {
                Keypair::from_str(str)
                    .map(|keypair| keypair.public_key().to_owned())
                    .unwrap_or_else(|_| self.keypair.public_key().to_owned())
            },
        )
    }
}

fn default_listen() -> String {
    "127.0.0.1:1680".to_string()
}

fn default_api() -> u16 {
    4467
}

#[derive(Debug)]
#[repr(u8)]
pub enum StakingMode {
    DataOnly = 0,
    Light = 1,
    Full = 2,
}

impl From<GatewayStakingMode> for StakingMode {
    fn from(v: GatewayStakingMode) -> Self {
        match v {
            GatewayStakingMode::Dataonly => StakingMode::DataOnly,
            GatewayStakingMode::Full => StakingMode::Full,
            GatewayStakingMode::Light => StakingMode::Light,
        }
    }
}

impl From<&StakingMode> for GatewayStakingMode {
    fn from(v: &StakingMode) -> Self {
        match v {
            StakingMode::DataOnly => GatewayStakingMode::Dataonly,
            StakingMode::Full => GatewayStakingMode::Full,
            StakingMode::Light => GatewayStakingMode::Light,
        }
    }
}

impl fmt::Display for StakingMode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StakingMode::DataOnly => f.write_str("dataonly"),
            StakingMode::Full => f.write_str("full"),
            StakingMode::Light => f.write_str("light"),
        }
    }
}

impl FromStr for StakingMode {
    type Err = Error;
    fn from_str(v: &str) -> Result<Self> {
        match v.to_lowercase().as_ref() {
            "light" => Ok(Self::Light),
            "full" => Ok(Self::Full),
            "dataonly" => Ok(Self::DataOnly),
            _ => Err(Error::custom(format!("invalid staking mode {v}"))),
        }
    }
}

pub mod log_level {
    use serde::de::{self, Deserialize, Deserializer, Visitor};
    use std::fmt;

    #[derive(Debug, Clone, Copy)]
    pub struct Level(slog::Level);

    impl AsRef<slog::Level> for Level {
        fn as_ref(&self) -> &slog::Level {
            &self.0
        }
    }

    impl From<Level> for slog::Level {
        fn from(v: Level) -> Self {
            v.0
        }
    }

    impl<'de> Deserialize<'de> for Level {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct LevelVisitor;

            impl<'de> Visitor<'de> for LevelVisitor {
                type Value = Level;
                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("log level")
                }
                fn visit_str<E>(self, value: &str) -> std::result::Result<Level, E>
                where
                    E: de::Error,
                {
                    value
                        .parse()
                        .map(Level)
                        .map_err(|_| de::Error::custom(format!("invalid log level \"{value}\"")))
                }
            }

            deserializer.deserialize_str(LevelVisitor)
        }
    }

    // pub fn deserialize<'de, D>(d: D) -> std::result::Result<slog::Level, D::Error>
    // where
    //     D: Deserializer<'de>,
    // {
    //     let s = String::deserialize(d)?;
    //     s.parse()
    //         .map_err(|_| de::Error::custom(format!("invalid log level \"{s}\"")))
    // }
}

pub mod log_method {
    use serde::de::{self, Deserialize, Deserializer, Visitor};
    use std::fmt;

    /// The method to use for logging.
    #[derive(Debug)]
    pub enum LogMethod {
        /// Display logging information on stdout
        Stdio,
        /// Send logging information to syslog
        Syslog,
    }

    impl<'de> Deserialize<'de> for LogMethod {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct LogMethodVisitor;

            impl<'de> Visitor<'de> for LogMethodVisitor {
                type Value = LogMethod;
                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("log method")
                }
                fn visit_str<E>(self, value: &str) -> std::result::Result<LogMethod, E>
                where
                    E: de::Error,
                {
                    let method = match value.to_lowercase().as_str() {
                        "stdio" => LogMethod::Stdio,
                        "syslog" => LogMethod::Syslog,
                        unsupported => {
                            return Err(de::Error::custom(format!(
                                "unsupported log method: \"{unsupported}\""
                            )))
                        }
                    };
                    Ok(method)
                }
            }

            deserializer.deserialize_str(LogMethodVisitor)
        }
    }
}
