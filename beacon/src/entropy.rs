use crate::{Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use helium_proto::EntropyReportV1;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::time::{SystemTime, UNIX_EPOCH};

pub const LOCAL_ENTROPY_SIZE: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entropy {
    #[serde(default = "default_version")]
    pub version: u32,
    pub timestamp: i64,
    #[serde(with = "serde_base64")]
    pub data: Vec<u8>,
}

impl Entropy {
    /// Construct entropy from a local system source. The timestamp and version
    /// of local entropy is always 0.
    pub fn local() -> Result<Self> {
        let mut local_entropy = vec![0u8; LOCAL_ENTROPY_SIZE];
        OsRng.fill_bytes(&mut local_entropy);
        Ok(Self {
            version: 0,
            timestamp: 0,
            data: local_entropy,
        })
    }

    pub fn from_data(data: Vec<u8>) -> Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(Error::from)?
            .as_secs() as i64;
        Ok(Self {
            version: 1, // marked as local
            timestamp,
            data,
        })
    }

    pub(crate) fn digest<D: Digest>(&self, state: &mut D) {
        state.update(&self.data);
        state.update(self.timestamp.to_le_bytes());
    }
}

fn default_version() -> u32 {
    0
}

mod serde_base64 {
    use base64::{engine::general_purpose::STANDARD, Engine};

    pub fn serialize<T, S>(key: &T, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        T: AsRef<[u8]>,
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(key.as_ref()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<Vec<u8>, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        struct _Visitor;

        impl<'de> serde::de::Visitor<'de> for _Visitor {
            type Value = Vec<u8>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("base64 entropy")
            }
            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                STANDARD
                    .decode(value)
                    .map_err(|err| serde::de::Error::custom(err.to_string()))
            }
        }

        deserializer.deserialize_str(_Visitor)
    }
}

impl std::fmt::Display for Entropy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&STANDARD.encode(&self.data))
    }
}

impl From<EntropyReportV1> for Entropy {
    fn from(value: EntropyReportV1) -> Self {
        Self {
            version: value.version,
            timestamp: value.timestamp as i64,
            data: value.data,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_serde() {
        const TIMESTAMP: u64 = 1663702455;
        const DATA: &str = "CE98+3O9JaKJYQqNO7vCF94iOVasA/TaWfdcpvLmcWs=";
        const VERSION: u32 = 0;
        const JSON_STR: &str = r#"{
            "version": 0,
            "timestamp": 1663702455,
            "data": "CE98+3O9JaKJYQqNO7vCF94iOVasA/TaWfdcpvLmcWs="
        }"#;

        let entropy: Entropy = serde_json::from_str(JSON_STR).expect("deserialized entropy");
        let ser_entropy = serde_json::to_value(&entropy).expect("serialized entropy");
        assert_eq!(VERSION, *ser_entropy.get("version").unwrap());
        assert_eq!(TIMESTAMP, *ser_entropy.get("timestamp").unwrap());
        assert_eq!(DATA, *ser_entropy.get("data").unwrap());
        assert_eq!(DATA, entropy.to_string());
    }
}
