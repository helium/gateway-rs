use crate::{Error, Result};
use helium_proto::{
    services::iot_config::GatewayRegionParamsResV1, BlockchainRegionParamV1,
    BlockchainRegionParamsV1, DataRate, GatewayRegionParamsRespV1,
    GatewayRegionParamsStreamedRespV1, Message, Region as ProtoRegion, RegionSpreading,
};
use rust_decimal::prelude::{Decimal, ToPrimitive};
use serde::{de, Deserialize, Deserializer};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region(ProtoRegion);

impl From<Region> for ProtoRegion {
    fn from(v: Region) -> Self {
        v.0
    }
}

impl From<ProtoRegion> for Region {
    fn from(value: ProtoRegion) -> Self {
        Self(value)
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

impl Default for Region {
    fn default() -> Self {
        Region(ProtoRegion::Unknown)
    }
}

impl Region {
    pub fn from_i32(v: i32) -> Result<Self> {
        ProtoRegion::from_i32(v)
            .map(Self)
            .ok_or_else(|| Error::unsupported_region(v))
    }

    pub fn is_unknown(&self) -> bool {
        self.0 == ProtoRegion::Unknown
    }
}

#[derive(Debug, Clone, Default)]
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

impl PartialEq for RegionParams {
    fn eq(&self, other: &Self) -> bool {
        self.gain.eq(&other.gain) && self.region.eq(&other.region) && self.params.eq(&other.params)
    }
}

impl TryFrom<GatewayRegionParamsStreamedRespV1> for RegionParams {
    type Error = Error;
    fn try_from(value: GatewayRegionParamsStreamedRespV1) -> Result<Self> {
        let region = Region::from_i32(value.region)?;
        let params = value
            .params
            .ok_or_else(Error::no_region_params)?
            .region_params;
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
        let params = value
            .params
            .ok_or_else(Error::no_region_params)?
            .region_params;
        Ok(Self {
            gain: Decimal::new(value.gain as i64, 1),
            params,
            region,
        })
    }
}

impl TryFrom<GatewayRegionParamsResV1> for RegionParams {
    type Error = Error;
    fn try_from(value: GatewayRegionParamsResV1) -> Result<Self> {
        let region = Region::from_i32(value.region)?;
        let params = value
            .params
            .ok_or_else(Error::no_region_params)?
            .region_params;
        Ok(Self {
            gain: Decimal::new(value.gain as i64, 1),
            params,
            region,
        })
    }
}

impl From<Region> for RegionParams {
    fn from(region: Region) -> Self {
        Self {
            region,
            gain: 0.into(),
            params: vec![],
        }
    }
}

impl RegionParams {
    pub fn from_bytes(region: Region, gain: u64, data: &[u8]) -> Result<Self> {
        let params = BlockchainRegionParamsV1::decode(data)?.region_params;
        let gain = Decimal::new(gain as i64, 1);
        Ok(Self {
            region,
            gain,
            params,
        })
    }

    pub fn is_unknown(&self) -> bool {
        self.region.is_unknown()
    }
}

impl RegionParams {
    pub fn check_valid(&self) -> Result {
        if self.is_unknown() || self.params.is_empty() {
            return Err(Error::no_region_params());
        }
        Ok(())
    }

    pub fn max_eirp(&self) -> Result<Decimal> {
        self.check_valid()?;
        self.params
            .iter()
            .max_by_key(|p| p.max_eirp)
            .ok_or_else(Error::no_region_params)
            .map(|v| Decimal::new(v.max_eirp as i64, 1))
    }

    pub fn max_conducted_power(&self) -> Result<u32> {
        self.max_eirp().and_then(|max_eirp| {
            (max_eirp - self.gain)
                .trunc()
                .to_u32()
                .ok_or_else(Error::invalid_conducted_power)
        })
    }

    /// Selects the best spreading factor for the given packet size.
    ///
    /// NOTE: This implementation matches the legacy Helium miner behavior in
    /// how it selects the tagged spreading from the region parameters. This
    /// legacy tagged spreading behavior assumes the unsorted ordering as stored
    /// in the region params.
    pub fn select_spreading(&self, packet_size: usize) -> Result<RegionSpreading> {
        // The spreading does not change per channel frequency So just get one
        // and do selection depending on max_packet_size
        self.params
            .first()
            // ensure region params themselves are available
            .ok_or_else(Error::no_region_params)
            // Ensure spreading parameters are available
            .and_then(|params| {
                params
                    .spreading
                    .as_ref()
                    .ok_or_else(Error::no_region_spreading)
            })
            // Find the first tagged spreading that can manage the given
            // packet size and extract the region spreading
            .and_then(|spreading| {
                spreading
                    .tagged_spreading
                    .iter()
                    .find(|tagged_spreading| {
                        tagged_spreading.max_packet_size as usize >= packet_size
                    })
                    .map(|tagged_spreading| tagged_spreading.region_spreading)
                    .ok_or_else(|| Error::no_region_spreading_for_size(packet_size))
            })
            // Convert to RegionSpreading
            .and_then(|region_spreading| {
                RegionSpreading::from_i32(region_spreading)
                    .ok_or_else(|| Error::unsupported_region_spreading(region_spreading))
            })
    }

    /// Gets the bandwidth for teh region parameters if present.
    // The bandwidth does not change per channel frequency, so this gets the
    // first one
    pub fn bandwidth(&self) -> Result<u32> {
        self.params
            .first()
            // ensure region params themselves are available
            .ok_or_else(Error::no_region_params)
            .map(|params| params.bandwidth)
    }

    /// Convenience function to select a spreading and bandwidth for a given
    /// packet size and convert to a DataRate
    pub fn select_datarate(&self, packet_size: usize) -> Result<DataRate> {
        let spreading = self.select_spreading(packet_size)?.as_str_name();
        let bandwidth = self.bandwidth()? / 1000; // in khz

        DataRate::from_str(&format!("{spreading}BW{bandwidth}")).map_err(|_| Error::no_data_rate())
    }
}

impl std::fmt::Display for RegionParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.region.fmt(f)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const EU868_PARAMS: &[u8] = &[
        10, 35, 8, 224, 202, 187, 157, 3, 16, 200, 208, 7, 24, 161, 1, 34, 20, 10, 4, 8, 6, 16, 65,
        10, 5, 8, 3, 16, 129, 1, 10, 5, 8, 2, 16, 238, 1, 10, 35, 8, 160, 229, 199, 157, 3, 16,
        200, 208, 7, 24, 161, 1, 34, 20, 10, 4, 8, 6, 16, 65, 10, 5, 8, 3, 16, 129, 1, 10, 5, 8, 2,
        16, 238, 1, 10, 35, 8, 224, 255, 211, 157, 3, 16, 200, 208, 7, 24, 161, 1, 34, 20, 10, 4,
        8, 6, 16, 65, 10, 5, 8, 3, 16, 129, 1, 10, 5, 8, 2, 16, 238, 1, 10, 35, 8, 160, 154, 224,
        157, 3, 16, 200, 208, 7, 24, 161, 1, 34, 20, 10, 4, 8, 6, 16, 65, 10, 5, 8, 3, 16, 129, 1,
        10, 5, 8, 2, 16, 238, 1, 10, 35, 8, 224, 180, 236, 157, 3, 16, 200, 208, 7, 24, 161, 1, 34,
        20, 10, 4, 8, 6, 16, 65, 10, 5, 8, 3, 16, 129, 1, 10, 5, 8, 2, 16, 238, 1, 10, 35, 8, 160,
        207, 248, 157, 3, 16, 200, 208, 7, 24, 161, 1, 34, 20, 10, 4, 8, 6, 16, 65, 10, 5, 8, 3,
        16, 129, 1, 10, 5, 8, 2, 16, 238, 1, 10, 35, 8, 224, 233, 132, 158, 3, 16, 200, 208, 7, 24,
        161, 1, 34, 20, 10, 4, 8, 6, 16, 65, 10, 5, 8, 3, 16, 129, 1, 10, 5, 8, 2, 16, 238, 1, 10,
        35, 8, 160, 132, 145, 158, 3, 16, 200, 208, 7, 24, 161, 1, 34, 20, 10, 4, 8, 6, 16, 65, 10,
        5, 8, 3, 16, 129, 1, 10, 5, 8, 2, 16, 238, 1,
    ];

    #[test]
    fn test_select_datarate() {
        let region = ProtoRegion::Eu868.into();
        let params = RegionParams::from_bytes(region, 12, EU868_PARAMS).expect("region params");
        assert_eq!(
            DataRate::Sf12bw125,
            params.select_datarate(30).expect("datarate")
        );
        assert_eq!(
            DataRate::Sf9bw125,
            params.select_datarate(90).expect("datarate")
        );
        assert_eq!(
            DataRate::Sf8bw125,
            params.select_datarate(130).expect("datarate")
        );
        assert!(params.select_datarate(300).is_err());
    }
}
