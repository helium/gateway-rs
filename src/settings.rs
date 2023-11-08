use crate::{api::GatewayStakingMode, KeyedUri, Keypair, PublicKey, Region, Result};
use config::{Config, Environment, File};
use http::uri::Uri;
use serde::Deserialize;
use std::{fmt, path::Path, str::FromStr, sync::Arc};

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
    pub api: ListenAddress,
    /// The location of the keypair binary file for the gateway. If the keyfile
    /// is not found there a new one is generated and saved in that location.
    pub keypair: Arc<Keypair>,
    /// The location of the onboarding keypair binary file for the gateway. If
    /// the keyfile is not found there a new one is generated and saved in that
    /// location.
    pub onboarding: Option<String>,
    /// The lorawan region to use. This value should line up with the configured
    /// region of the semtech packet forwarder. Defaults to the "UNKNOWN" region
    /// which will delay poc and packet activity for a short duration until the
    /// asserted location/region is fetched.
    #[serde(default)]
    pub region: Region,
    /// Log settings
    pub log: LogSettings,
    /// The config service to use for region and other config settings
    pub config: KeyedUri,
    /// The packet router to deliver all packets when packet router is active.
    pub router: RouterSettings,
    /// Proof-of-coverage (PoC) settings.
    pub poc: PocSettings,
}

/// Settings for log method and level to be used by the running service.
#[derive(Debug, Deserialize)]
pub struct LogSettings {
    /// Log level to show (default info)
    pub level: log_level::Level,

    /// Whehter to show timestamps in the stdio output stream (default false)
    pub timestamp: bool,
}

impl LogSettings {
    pub fn time_formatter(&self) -> impl tracing_subscriber::fmt::time::FormatTime {
        TimeFormatter {
            timestamp: self.timestamp,
            time: tracing_subscriber::fmt::time(),
        }
    }
}

struct TimeFormatter {
    timestamp: bool,
    time: tracing_subscriber::fmt::time::SystemTime,
}

impl tracing_subscriber::fmt::time::FormatTime for TimeFormatter {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> fmt::Result {
        if self.timestamp {
            self.time.format_time(w)
        } else {
            Ok(())
        }
    }
}

/// Settings for proof-of-coverage (PoC).
#[derive(Debug, Deserialize, Clone)]
pub struct PocSettings {
    // Enable/disable poc related activities (baecon/witness)
    #[serde(default)]
    pub disable: bool,
    /// Entropy URL.
    #[serde(with = "http_serde::uri")]
    pub entropy_uri: Uri,
    /// Remote ingestor URL.
    #[serde(with = "http_serde::uri")]
    pub ingest_uri: Uri,
    /// Beacon interval in seconds. Defaults to 6 hours. Note that the rate of
    /// beacons is verified by the oracle so increasing this number will not
    /// increase rewards
    #[serde(default = "default_poc_interval")]
    pub interval: u64,
}

/// Settings for packet routing
#[derive(Debug, Deserialize, Clone)]
pub struct RouterSettings {
    #[serde(with = "http_serde::uri")]
    pub uri: Uri,
    // Maximum number of packets to queue up for the packet router
    pub queue: u16,
}

impl Settings {
    /// Settings are loaded from the file in the given path.
    ///
    /// Environemnt overrides have the same name as the entries in the settings
    /// file in uppercase and prefixed with "GW_". For example "GW_KEY" will
    /// override the key file location.
    pub fn new(path: &Path) -> Result<Self> {
        Config::builder()
            // Source settings file
            .add_source(File::with_name(path.to_str().expect("file name")).required(false))
            // Add in settings from the environment (with a prefix of APP)
            // Eg.. `GW_DEBUG=1 ./target/app` would set the `debug` key
            .add_source(Environment::with_prefix("gw").separator("_"))
            .build()
            .and_then(|config| config.try_deserialize())
            .map_err(|e| e.into())
    }

    /// Returns the onboarding key for this gateway. The onboarding key is
    /// determined by the onboarding setting. If the onbaording setting is not
    /// present or there is any error retrieving the onboarding key from the
    /// configured setting the public key of the gateway is returned.
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

fn default_api() -> ListenAddress {
    ListenAddress::Address("127.0.0.1:4467".to_string())
}

fn default_poc_interval() -> u64 {
    // every 6 hours
    6 * 3600
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Copy, clap::ValueEnum)]
#[clap(rename_all = "lower")]
#[repr(u8)]
pub enum StakingMode {
    DataOnly = 0,
    Full = 2,
}

impl From<GatewayStakingMode> for StakingMode {
    fn from(v: GatewayStakingMode) -> Self {
        match v {
            GatewayStakingMode::Dataonly => StakingMode::DataOnly,
            GatewayStakingMode::Full => StakingMode::Full,
            // Light gateways were never implemented but were defined in staking
            // modes. They're equivalent to full hotspots
            GatewayStakingMode::Light => StakingMode::Full,
        }
    }
}

impl From<&StakingMode> for GatewayStakingMode {
    fn from(v: &StakingMode) -> Self {
        match v {
            StakingMode::DataOnly => GatewayStakingMode::Dataonly,
            StakingMode::Full => GatewayStakingMode::Full,
        }
    }
}

impl fmt::Display for StakingMode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use clap::ValueEnum;
        self.to_possible_value()
            .expect("no values are skipped")
            .get_name()
            .fmt(f)
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum ListenAddress {
    Port(u16),
    Address(String),
}

impl TryFrom<&ListenAddress> for std::net::SocketAddr {
    type Error = crate::Error;
    fn try_from(value: &ListenAddress) -> std::result::Result<Self, Self::Error> {
        fn local_addr_from_port(v: &u16) -> String {
            format!("127.0.0.1:{v}")
        }
        match value {
            ListenAddress::Address(str) => {
                if let Ok(v) = str.parse::<u16>() {
                    Ok(local_addr_from_port(&v).parse()?)
                } else {
                    Ok(str.parse()?)
                }
            }
            ListenAddress::Port(v) => Ok(local_addr_from_port(v).parse()?),
        }
    }
}

impl TryFrom<&ListenAddress> for http::Uri {
    type Error = crate::Error;
    fn try_from(value: &ListenAddress) -> std::result::Result<Self, Self::Error> {
        fn local_uri_from_port(v: &u16) -> String {
            format!("http://127.0.0.1:{v}")
        }
        match value {
            ListenAddress::Address(str) => {
                if let Ok(v) = str.parse::<u16>() {
                    Ok(local_uri_from_port(&v).parse()?)
                } else {
                    Ok(format!("http://{str}").parse()?)
                }
            }
            ListenAddress::Port(v) => Ok(local_uri_from_port(v).parse()?),
        }
    }
}

pub mod log_level {
    use serde::de::{self, Deserialize, Deserializer, Visitor};
    use std::fmt;

    #[derive(Debug, Clone, Copy)]
    pub struct Level(tracing::Level);

    impl std::fmt::Display for Level {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }

    impl From<Level> for tracing_subscriber::filter::LevelFilter {
        fn from(value: Level) -> Self {
            Self::from(value.0)
        }
    }

    impl From<tracing::Level> for Level {
        fn from(value: tracing::Level) -> Self {
            Self(value)
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
}

#[cfg(test)]
mod test {
    use super::*;
    use std::net::SocketAddr;

    #[test]
    fn listen_addr() {
        assert_eq!(
            SocketAddr::try_from(&ListenAddress::Port(4468)).expect("socket addr from port"),
            "127.0.0.1:4468".parse().expect("socket addr")
        );
        assert_eq!(
            SocketAddr::try_from(&ListenAddress::Address("4468".to_string()))
                .expect("socket addr from port str"),
            "127.0.0.1:4468".parse().expect("socket addr")
        );
        assert_eq!(
            SocketAddr::try_from(&ListenAddress::Address("1.2.3.4:4468".to_string()))
                .expect("socket addr from addr string"),
            "1.2.3.4:4468".parse().expect("socket addr")
        );

        // Now try URI form
        assert_eq!(
            Uri::try_from(&ListenAddress::Port(4468)).expect("uri from port"),
            Uri::from_static("http://127.0.0.1:4468")
        );
        assert_eq!(
            Uri::try_from(&ListenAddress::Address("4468".to_string())).expect("uri from port str"),
            Uri::from_static("http://127.0.0.1:4468")
        );
        assert_eq!(
            Uri::try_from(&ListenAddress::Address("1.2.3.4:4468".to_string()))
                .expect("uri from addr string"),
            Uri::from_static("http://1.2.3.4:4468")
        );
    }
}
