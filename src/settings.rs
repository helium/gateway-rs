use anyhow::{anyhow, Context, Result};
use config::{Config, Environment, File};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub semtech_udp: SemtechUdp,
}

#[derive(Debug, Deserialize)]
pub struct SemtechUdp {
    pub listen_addr: String,
}

impl Settings {
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let mut c = Config::new();

        let _ = match path {
            Some(p) => {
                let path_str = p
                    .to_str()
                    .ok_or_else(|| anyhow!("Invalid path {}", p.display()))?;
                c.merge(File::with_name(&path_str))?
            }
            None => {
                // Start off by merging in the "default" configuration file
                c.merge(File::with_name("config/default"))?;

                // Add in a local configuration file
                // This file shouldn't be checked in to git
                c.merge(File::with_name("config/local").required(false))?
            }
        };
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `GW_DEBUG=1 ./target/app` would set the `debug` key
        c.merge(Environment::with_prefix("gw"))?;

        c.try_into()
            .with_context(|| "Failed to read config file(s)")
    }
}
