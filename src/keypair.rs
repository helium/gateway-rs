use crate::*;
use helium_crypto::{KeyTag, KeyType, Network};
use rand::rngs::OsRng;
use serde::{de, Deserialize, Deserializer};
use std::{convert::TryFrom, fs, path, sync::Arc};

pub type Keypair = helium_crypto::Keypair;
pub type PublicKey = helium_crypto::PublicKey;

pub fn load_from_file(path: &str) -> error::Result<Keypair> {
    let data = fs::read(path)?;
    Ok(Keypair::try_from(&data[..])?)
}

pub fn save_to_file(keypair: &Keypair, path: &str) -> Result {
    if let Some(parent) = path::PathBuf::from(path).parent() {
        fs::create_dir_all(parent)?;
    };
    fs::write(path, &keypair.to_bytes())?;
    Ok(())
}

pub fn deserialize<'de, D>(d: D) -> std::result::Result<Arc<Keypair>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    match load_from_file(&s) {
        Ok(k) => Ok(Arc::new(k)),
        Err(Error::IO(io_error)) if io_error.kind() == std::io::ErrorKind::NotFound => {
            let new_key = Keypair::generate(
                KeyTag {
                    network: Network::MainNet,
                    key_type: KeyType::Ed25519,
                },
                &mut OsRng,
            );
            save_to_file(&new_key, &s).map_err(|e| {
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
