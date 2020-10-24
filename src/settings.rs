use crate::{key, result::Result};
use config::{Config, Environment, File};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct Settings {
    pub listen_addr: String,
    pub key_file: String,
}

impl Settings {
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let mut c = Config::new();
        c.set_default("key_file", "/etc/gateway/gateway_key.pem")?;
        c.set_default("listen_addr", "127.0.0.1:1680")?;
        if let Some(p) = path {
            let path_str = p.to_str().unwrap();
            c.merge(File::with_name(&path_str))?;
        }
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `GW_DEBUG=1 ./target/app` would set the `debug` key
        c.merge(Environment::with_prefix("gw"))?;
        c.try_into().map_err(|e| e.into())
    }

    pub fn key(&self) -> Result<key::Key> {
        match key::Key::load(&self.key_file) {
            Ok(k) => Ok(k),
            Err(_) => {
                let new_key = key::Key::generate()?;
                new_key.save(&self.key_file)?;
                Ok(new_key)
            }
        }
    }

    pub fn listen_addr(&self) -> Result<SocketAddr> {
        Ok(self.listen_addr.parse()?)
    }
}
