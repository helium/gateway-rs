use crate::{PublicKey, Result};
use http::Uri;
use serde::Deserialize;
use std::{fmt, str::FromStr, sync::Arc};

/// A URI that has an associated public key
#[derive(Clone, Deserialize, Eq)]
pub struct KeyedUri {
    #[serde(with = "http_serde::uri")]
    pub uri: Uri,
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

impl TryFrom<helium_proto::services::local::KeyedUri> for KeyedUri {
    type Error = crate::Error;
    fn try_from(v: helium_proto::services::local::KeyedUri) -> Result<Self> {
        let result = Self {
            uri: http::Uri::from_str(&v.uri)?,
            pubkey: Arc::new(helium_crypto::PublicKey::from_bytes(v.address)?),
        };
        Ok(result)
    }
}

impl From<KeyedUri> for helium_proto::services::local::KeyedUri {
    fn from(v: KeyedUri) -> Self {
        helium_proto::services::local::KeyedUri {
            address: v.pubkey.to_vec(),
            uri: v.uri.to_string(),
        }
    }
}

impl TryFrom<helium_proto::RoutingAddress> for KeyedUri {
    type Error = crate::Error;
    fn try_from(v: helium_proto::RoutingAddress) -> Result<Self> {
        let result = Self {
            uri: http::Uri::from_str(&String::from_utf8_lossy(&v.uri))?,
            pubkey: Arc::new(helium_crypto::PublicKey::from_bytes(v.pub_key)?),
        };
        Ok(result)
    }
}
