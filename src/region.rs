use crate::{Error, Result};
use helium_proto::Region as ProtoRegion;
use serde::{de, Deserialize, Deserializer};
use std::fmt;

#[derive(Debug, Clone, Copy)]
pub struct Region(ProtoRegion);

impl From<Region> for ProtoRegion {
    fn from(v: Region) -> Self {
        v.0
    }
}

impl<'de> Deserialize<'de> for Region {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RegionVisitor;

        impl<'de> de::Visitor<'de> for RegionVisitor {
            type Value = Region;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("region string")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Region, E>
            where
                E: de::Error,
            {
                let proto_region = match value {
                    "US915" => ProtoRegion::Us915,
                    "EU868" => ProtoRegion::Eu868,
                    "EU433" => ProtoRegion::Eu433,
                    "CN470" => ProtoRegion::Cn470,
                    "CN779" => ProtoRegion::Cn779,
                    "AU915" => ProtoRegion::Au915,
                    "AS923_1" => ProtoRegion::As9231,
                    "AS923_2" => ProtoRegion::As9232,
                    "AS923_3" => ProtoRegion::As9233,
                    "AS923_4" => ProtoRegion::As9234,
                    "KR920" => ProtoRegion::Kr920,
                    "IN865" => ProtoRegion::In865,
                    unsupported => {
                        return Err(de::Error::custom(format!(
                            "unsupported region: {unsupported}"
                        )))
                    }
                };
                Ok(Region(proto_region))
            }
        }

        deserializer.deserialize_str(RegionVisitor)
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            ProtoRegion::Us915 => f.write_str("US915"),
            ProtoRegion::Eu868 => f.write_str("EU868"),
            ProtoRegion::Eu433 => f.write_str("EU433"),
            ProtoRegion::Cn470 => f.write_str("CN470"),
            ProtoRegion::Cn779 => f.write_str("CN779"),
            ProtoRegion::Au915 => f.write_str("AU915"),
            ProtoRegion::As9231 => f.write_str("AS923_1"),
            ProtoRegion::As9232 => f.write_str("AS923_2"),
            ProtoRegion::As9233 => f.write_str("AS923_3"),
            ProtoRegion::As9234 => f.write_str("AS923_4"),
            ProtoRegion::Kr920 => f.write_str("KR920"),
            ProtoRegion::In865 => f.write_str("IN865"),
        }
    }
}

impl From<Region> for i32 {
    fn from(region: Region) -> Self {
        region.0.into()
    }
}

impl From<&Region> for i32 {
    fn from(region: &Region) -> Self {
        region.0.into()
    }
}

impl Region {
    pub fn from_i32(v: i32) -> Result<Self> {
        ProtoRegion::from_i32(v)
            .map(Self)
            .ok_or_else(|| Error::custom(format!("unsupported region {v}")))
    }
}
