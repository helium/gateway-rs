use crate::{error::RegionError, Error, Result};
use helium_proto::{
    BlockchainRegionParamV1, GatewayRegionParamsRespV1, GatewayRegionParamsStreamedRespV1,
    Region as ProtoRegion,
};
use rust_decimal::Decimal;
use serde::{de, Deserialize, Deserializer};
use std::{fmt, str::FromStr};

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
                let proto_region = ProtoRegion::from_str(value)
                    .map_err(|_| de::Error::custom(format!("unsupported region: {value}")))?;
                Ok(Region(proto_region))
            }
        }

        deserializer.deserialize_str(RegionVisitor)
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
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

impl slog::Value for Region {
    fn serialize(
        &self,
        _record: &slog::Record,
        key: slog::Key,
        serializer: &mut dyn slog::Serializer,
    ) -> slog::Result {
        serializer.emit_str(key, &self.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct RegionParams {
    pub gain: Decimal,
    pub region: Region,
    pub params: Vec<BlockchainRegionParamV1>,
}

impl AsRef<[BlockchainRegionParamV1]> for RegionParams {
    fn as_ref(&self) -> &[BlockchainRegionParamV1] {
        self.params.as_ref()
    }
}

impl TryFrom<GatewayRegionParamsStreamedRespV1> for RegionParams {
    type Error = Error;
    fn try_from(value: GatewayRegionParamsStreamedRespV1) -> Result<Self> {
        let region = Region::from_i32(value.region)?;
        let params = if let Some(params) = value.params {
            params.region_params
        } else {
            return Err(RegionError::no_region_params());
        };
        Ok(Self {
            gain: Decimal::new(value.gain as i64, 1),
            params,
            region,
        })
    }
}

impl TryFrom<GatewayRegionParamsRespV1> for RegionParams {
    type Error = Error;
    fn try_from(value: GatewayRegionParamsRespV1) -> Result<Self> {
        let region = Region::from_i32(value.region)?;
        let params = if let Some(params) = value.params {
            params.region_params
        } else {
            return Err(RegionError::no_region_params());
        };
        Ok(Self {
            gain: Decimal::new(value.gain as i64, 1),
            params,
            region,
        })
    }
}

impl RegionParams {
    pub fn max_eirp(&self) -> Option<Decimal> {
        self.params
            .iter()
            .max_by_key(|p| p.max_eirp)
            .map(|v| Decimal::new(v.max_eirp as i64, 1))
    }

    pub fn tx_power(&self) -> Option<u32> {
        use rust_decimal::prelude::ToPrimitive;
        self.max_eirp()
            .and_then(|max_eirp| (max_eirp - self.gain).trunc().to_u32())
    }

    pub fn to_string(v: &Option<Self>) -> String {
        match v {
            None => "none".to_string(),
            Some(params) => params.region.to_string(),
        }
    }
}
