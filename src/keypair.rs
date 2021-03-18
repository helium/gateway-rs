use crate::*;
use helium_crypto::{ecc_compact, Error as CryptoError, KeyType};
use std::{convert::TryFrom, fs, path};

pub type Keypair = helium_crypto::ecc_compact::Keypair;
pub type PublicKey = helium_crypto::PublicKey;

pub fn load_from_file(path: &str) -> error::Result<Keypair> {
    let data = fs::read(path)?;
    match KeyType::try_from(data[0])? {
        KeyType::EccCompact => Ok(ecc_compact::Keypair::try_from(&data[..])?),
        _ => Err(CryptoError::invalid_keytype(data[0]).into()),
    }
}

pub fn save_to_file(keypair: &Keypair, path: &str) -> Result {
    if let Some(parent) = path::PathBuf::from(path).parent() {
        fs::create_dir_all(parent)?;
    };
    fs::write(path, &keypair.to_bytes())?;
    Ok(())
}
