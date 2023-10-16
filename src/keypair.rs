use crate::{DecodeError, Error, Result};
#[cfg(feature = "ecc608")]
use helium_crypto::ecc608;
#[cfg(feature = "tpm")]
use helium_crypto::tpm;
use helium_crypto::{KeyTag, KeyType, Network};
use http::Uri;
use rand::rngs::OsRng;
use serde::{de, Deserializer};
#[cfg(feature = "ecc608")]
use std::path::Path;
use std::{collections::HashMap, convert::TryFrom, fmt, fs, io, path, str::FromStr};
use tonic::async_trait;

#[derive(Debug)]
pub struct Keypair(helium_crypto::Keypair);
pub type PublicKey = helium_crypto::PublicKey;

#[async_trait]
pub trait Sign {
    async fn sign<K>(&mut self, keypair: K) -> Result
    where
        K: AsRef<Keypair> + std::marker::Send + 'static;
}

pub trait Verify {
    fn verify(&self, pub_key: &crate::PublicKey) -> Result;
}

macro_rules! uri_error {
    ($format:expr) => {
        DecodeError::keypair_uri(format!($format))
    };
    ($format:expr, $( $arg:expr ),+ ) => {
        DecodeError::keypair_uri(format!($format, $( $arg ),+))
    };
}

impl From<helium_crypto::Keypair> for Keypair {
    fn from(v: helium_crypto::Keypair) -> Self {
        Self(v)
    }
}

impl FromStr for Keypair {
    type Err = Error;
    fn from_str(str: &str) -> Result<Self> {
        let url: Uri = str
            .parse()
            .map_err(|err| uri_error!("invalid keypair url \"{str}\": {err:?}"))?;
        match url.scheme_str() {
            Some("file") | None => match Self::load_from_file(url.path()) {
                Ok(k) => Ok(k),
                Err(Error::IO(io_error)) if io_error.kind() == std::io::ErrorKind::NotFound => {
                    let args = KeypairArgs::from_uri(&url)?;
                    let network = args.get::<Network>("network", Network::MainNet)?;
                    let new_key: Keypair = helium_crypto::Keypair::generate(
                        KeyTag {
                            network,
                            key_type: KeyType::Ed25519,
                        },
                        &mut OsRng,
                    )
                    .into();
                    new_key.save_to_file(url.path()).map_err(|err| {
                        uri_error!("unable to save key file \"{}\": {err:?}", url.path())
                    })?;
                    Ok(new_key)
                }
                Err(err) => Err(uri_error!(
                    "unable to load key file \"{}\": {err:?}",
                    url.path()
                )),
            },
            #[cfg(feature = "ecc608")]
            Some("ecc") => {
                let args = KeypairArgs::from_uri(&url).map_err(DecodeError::keypair_uri)?;

                let bus_address = url.port_u16().unwrap_or(96);
                let slot = args.get::<u8>("slot", 0)?;
                let network = args.get("network", Network::MainNet)?;
                let path = url
                    .host()
                    .map(|dev| Path::new("/dev").join(dev))
                    .ok_or_else(|| uri_error!("missing ecc device path"))?;
                let keypair = ecc608::init(&path.to_string_lossy(), bus_address, None)
                    .map_err(|err| {
                        uri_error!(
                            "could not initialize ecc \"{}:{bus_address}\": {err:?}",
                            path.to_string_lossy()
                        )
                    })
                    .and_then(|_| {
                        ecc608::Keypair::from_slot(network, slot)
                            .map(helium_crypto::Keypair::from)
                            .map_err(|err| {
                                uri_error!("could not load ecc keypair in slot {slot}: {err:?}")
                            })
                    })?;
                Ok(keypair.into())
            }
            #[cfg(feature = "tpm")]
            Some("tpm") => {
                let args = KeypairArgs::from_uri(&url).map_err(DecodeError::keypair_uri)?;
                let network = args.get("network", Network::MainNet)?;
                let key_identifier = &url.path()[1..];
                let key_access = url.host().unwrap();

                let keypair = match key_access {
                    "esys" => tpm::KeypairHandle::from_key_handle(
                        network,
                        u32::from_str_radix(&key_identifier[2..], 16).unwrap(),
                    )
                    .map(helium_crypto::Keypair::from),
                    _ => Err(helium_crypto::Error::invalid_keytype_str(
                        "unknown tpm key access type",
                    )),
                }
                .map_err(|err| uri_error!("could not load tpm key {key_access}: {err:?}"))?;

                Ok(keypair.into())
            }
            Some(unknown) => Err(uri_error!("unkown keypair scheme: \"{unknown}\"")),
        }
    }
}

impl std::ops::Deref for Keypair {
    type Target = helium_crypto::Keypair;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Keypair {
    pub fn new() -> Self {
        let keypair = helium_crypto::Keypair::generate(
            KeyTag {
                network: Network::MainNet,
                key_type: KeyType::Ed25519,
            },
            &mut OsRng,
        );
        keypair.into()
    }

    pub fn load_from_file(path: &str) -> Result<Self> {
        let data = fs::read(path)?;
        Ok(helium_crypto::Keypair::try_from(&data[..])?.into())
    }

    pub fn save_to_file(&self, path: &str) -> io::Result<()> {
        if let Some(parent) = path::PathBuf::from(path).parent() {
            fs::create_dir_all(parent)?;
        };
        fs::write(path, self.0.to_vec())?;
        Ok(())
    }
}

impl Default for Keypair {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
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
            .unwrap_or_else(|| Ok(default))
            .map_err(|err| uri_error!("invalid uri argument for {name}: {err:?}"))
    }
}

impl<'de> de::Deserialize<'de> for Keypair {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct _Visitor;

        impl<'de> de::Visitor<'de> for _Visitor {
            type Value = Keypair;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("keypair uri")
            }
            fn visit_str<E>(self, value: &str) -> std::result::Result<Keypair, E>
            where
                E: de::Error,
            {
                Keypair::from_str(value).map_err(|err| de::Error::custom(err.to_string()))
            }
        }

        deserializer.deserialize_str(_Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypair_args() {
        let uri = &Uri::from_static("ecc://i2c-1:196?slot=22&network=testnet");
        let args = KeypairArgs::from_uri(uri).expect("keypair args");
        assert_eq!(22, args.get::<u8>("slot", 22).expect("slot"));
        assert_eq!(196, uri.port_u16().expect("uri port"));
        assert_eq!(
            Network::TestNet,
            args.get::<Network>("network", Network::MainNet)
                .expect("network")
        );
    }
}
