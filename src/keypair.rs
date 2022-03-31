use crate::*;
use helium_crypto::{ecc608, KeyTag, KeyType, Network};
use http::Uri;
use rand::rngs::OsRng;
use serde::{de, Deserialize, Deserializer};
use std::{collections::HashMap, convert::TryFrom, fs, io, path, path::Path, sync::Arc};

pub type Keypair = helium_crypto::Keypair;
pub type PublicKey = helium_crypto::PublicKey;

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

macro_rules! uri_error {
    ($format:expr) => {
        error::DecodeError::keypair_uri(format!($format))
    };
    ($format:expr, $( $arg:expr ),+ ) => {
        error::DecodeError::keypair_uri(format!($format, $( $arg ),+))
    };
}

pub fn from_str(str: &str) -> Result<Arc<Keypair>> {
    let url: Uri = str
        .parse()
        .map_err(|err| uri_error!("invalid keypair url \"{str}\": {err:?}"))?;
    match url.scheme_str() {
        Some("file") | None => match load_from_file(url.path()) {
            Ok(k) => Ok(Arc::new(k)),
            Err(Error::IO(io_error)) if io_error.kind() == std::io::ErrorKind::NotFound => {
                let args = KeypairArgs::from_uri(&url)?;
                let network = args.get::<Network>("network", Network::MainNet)?;
                let new_key = Keypair::generate(
                    KeyTag {
                        network,
                        key_type: KeyType::Ed25519,
                    },
                    &mut OsRng,
                );
                save_to_file(&new_key, url.path()).map_err(|err| {
                    uri_error!("unable to save key file \"{}\": {err:?}", url.path())
                })?;
                Ok(Arc::new(new_key))
            }
            Err(err) => Err(uri_error!(
                "unable to load key file \"{}\": {err:?}",
                url.path()
            )),
        },
        Some("ecc") => {
            let args = KeypairArgs::from_uri(&url).map_err(error::DecodeError::keypair_uri)?;

            let bus_address = url.port_u16().unwrap_or(96);
            let slot = args.get::<u8>("slot", 0)?;
            let network = args.get("network", Network::MainNet)?;
            let path = url
                .host()
                .map(|dev| Path::new("/dev").join(dev))
                .ok_or_else(|| uri_error!("missing ecc device path"))?;
            let keypair = ecc608::init(&path.to_string_lossy(), bus_address)
                .map_err(|err| {
                    uri_error!(
                        "could not initialize ecc \"{}:{bus_address}\": {err:?}",
                        path.to_string_lossy()
                    )
                })
                .and_then(|_| {
                    ecc608::Keypair::from_slot(network, slot).map_err(|err| {
                        uri_error!("could not load ecc keypair in slot {slot}: {err:?}")
                    })
                })?;
            Ok(Arc::new(keypair.into()))
        }
        Some(unknown) => Err(uri_error!("unkown keypair scheme: \"{unknown}\"")),
    }
}

struct KeypairArgs(HashMap<String, String>);

impl KeypairArgs {
    pub(crate) fn from_uri(url: &Uri) -> Result<Self> {
        let args = url
            .query()
            .map_or_else(
                || Ok(HashMap::new()),
                serde_urlencoded::from_str::<HashMap<String, String>>,
            )
            .map_err(|err| uri_error!("invalid keypair url \"{url}\": {err:?}"))?;
        Ok(Self(args))
    }

    pub fn get<T>(&self, name: &str, default: T) -> Result<T>
    where
        T: std::str::FromStr,
        <T as std::str::FromStr>::Err: std::fmt::Debug,
    {
        self.0
            .get(name)
            .map(|s| s.parse::<T>())
            .unwrap_or(Ok(default))
            .map_err(|err| uri_error!("invalid uri argument for {name}: {err:?}"))
    }
}

pub fn deserialize<'de, D>(d: D) -> std::result::Result<Arc<Keypair>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    from_str(&s).map_err(|err| de::Error::custom(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypair_args() {
        let args =
            KeypairArgs::from_uri(&Uri::from_static("ecc://i2c-1:96?slot=0&network=testnet"))
                .expect("keypair args");
        assert_eq!(0, args.get::<u8>("slot", 22).expect("slot"));
        assert_eq!(
            Network::TestNet,
            args.get::<Network>("network", Network::MainNet)
                .expect("network")
        );
    }
}
