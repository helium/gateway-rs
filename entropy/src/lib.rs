use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entropy {
    pub timestamp: i64,
    #[serde(with = "serde_base64")]
    pub data: Vec<u8>,
}

mod serde_base64 {
    pub fn serialize<T, S>(key: &T, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        T: AsRef<[u8]>,
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(&base64::encode(key.as_ref()))
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
                base64::decode(value).map_err(|err| serde::de::Error::custom(err.to_string()))
            }
        }

        deserializer.deserialize_str(_Visitor)
    }
}

impl std::fmt::Display for Entropy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&base64::encode(&self.data))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_serde() {
        const TIMESTAMP: u64 = 1663702455;
        const DATA: &str = "CE98+3O9JaKJYQqNO7vCF94iOVasA/TaWfdcpvLmcWs=";
        const JSON_STR: &str = r#"{
            "timestamp": 1663702455,
            "data": "CE98+3O9JaKJYQqNO7vCF94iOVasA/TaWfdcpvLmcWs="
        }"#;

        let entropy: Entropy = serde_json::from_str(JSON_STR).expect("deserialized entropy");
        let ser_entropy = serde_json::to_value(&entropy).expect("serialized entropy");
        assert_eq!(TIMESTAMP, *ser_entropy.get("timestamp").unwrap());
        assert_eq!(DATA, *ser_entropy.get("data").unwrap());
        assert_eq!(DATA, entropy.to_string());
    }
}
