use crate::PublicKey;
use http::Uri;
use serde::{de, Deserialize, Deserializer};
use std::{fmt, sync::Arc};

/// A URI that has an associated public key
#[derive(Clone, Deserialize, Eq)]
pub struct KeyedUri {
    #[serde(with = "http_serde::uri")]
    pub uri: Uri,
    #[serde(deserialize_with = "deserialize_pubkey")]
    pub pubkey: Arc<PublicKey>,
}

impl PartialEq for KeyedUri {
    fn eq(&self, other: &Self) -> bool {
        self.uri.eq(&other.uri) && self.pubkey.eq(&other.pubkey)
    }
}

impl fmt::Debug for KeyedUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyedUri")
            .field("uri", &self.uri)
            .field("pubkey", &self.pubkey.to_string())
            .finish()
    }
}

fn deserialize_pubkey<'de, D>(d: D) -> std::result::Result<Arc<PublicKey>, D::Error>
where
    D: Deserializer<'de>,
{
    let key_string = String::deserialize(d)?;
    match key_string.parse() {
        Ok(key) => Ok(Arc::new(key)),
        Err(err) => Err(de::Error::custom(format!("invalid pubkey: \"{}\"", err))),
    }
}

impl AsRef<Uri> for KeyedUri {
    fn as_ref(&self) -> &Uri {
        &self.uri
    }
}

impl AsRef<PublicKey> for KeyedUri {
    fn as_ref(&self) -> &PublicKey {
        &self.pubkey
    }
}
