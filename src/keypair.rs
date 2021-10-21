use crate::*;
use helium_crypto::{ecc608, KeyTag, KeyType, Network};
use rand::rngs::OsRng;
use serde::{de, Deserialize, Deserializer};
use std::{collections::HashMap, convert::TryFrom, fs, io, path, path::Path, sync::Arc};

pub type Keypair = helium_crypto::Keypair;
pub type PublicKey = helium_crypto::PublicKey;
use http::Uri;

pub fn load_from_file(path: &str) -> error::Result<Keypair> {
    let data = fs::read(path)?;
    Ok(Keypair::try_from(&data[..])?)
}

pub fn save_to_file(keypair: &Keypair, path: &str) -> io::Result<()> {
    if let Some(parent) = path::PathBuf::from(path).parent() {
        fs::create_dir_all(parent)?;
    };
    fs::write(path, &keypair.to_vec())?;
    Ok(())
}

macro_rules! de_error {
    ($format:expr) => {
        de::Error::custom($format)
    };
    ($format:expr, $( $arg:expr ),+ ) => {
        de::Error::custom(format!($format, $( $arg ),+))
    };
}

pub fn deserialize<'de, D>(d: D) -> std::result::Result<Arc<Keypair>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    let url: Uri = s
        .parse()
        .map_err(|err| de_error!("invalid keypair url \"{}\": {:?}", s, err))?;
    match url.scheme_str() {
        Some("file") | None => match load_from_file(url.path()) {
            Ok(k) => Ok(Arc::new(k)),
            Err(Error::IO(io_error)) if io_error.kind() == std::io::ErrorKind::NotFound => {
                let new_key = Keypair::generate(
                    KeyTag {
                        network: Network::MainNet,
                        key_type: KeyType::Ed25519,
                    },
                    &mut OsRng,
                );
                save_to_file(&new_key, url.path()).map_err(|err| {
                    de_error!("unable to save key file \"{}\": {:?}", url.path(), err)
                })?;
                Ok(Arc::new(new_key))
            }
            Err(err) => Err(de_error!(
                "unable to load key file \"{}\": {:?}",
                url.path(),
                err
            )),
        },
        Some("ecc") => {
            let args = url
                .query()
                .map_or_else(
                    || Ok(HashMap::new()),
                    serde_urlencoded::from_str::<HashMap<String, u8>>,
                )
                .map_err(|err| de_error!("invalid ecc bus options: {:?}", err))?;
            let bus_address = url.port_u16().unwrap_or(96);
            let slot = *args.get("slot").unwrap_or(&0);
            let path = url
                .host()
                .map(|dev| Path::new("/dev").join(dev))
                .ok_or_else(|| de_error!("missing ecc device path"))?;
            let keypair = ecc608::init(&path.to_string_lossy(), bus_address)
                .map_err(|err| {
                    de_error!(
                        "could not initialize ecc \"{}:{}\": {:?}",
                        path.to_string_lossy(),
                        bus_address,
                        err
                    )
                })
                .and_then(|_| {
                    ecc608::Keypair::from_slot(Network::MainNet, slot).map_err(|err| {
                        de_error!("could not load ecc keypair in slot {}: {:?}", slot, err)
                    })
                })?;
            Ok(Arc::new(keypair.into()))
        }
        Some(unknown) => Err(de_error!("unkown keypair scheme: \"{}\"", unknown)),
    }
}
