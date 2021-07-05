use crate::*;
use std::{convert::TryFrom, fs, path};

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
