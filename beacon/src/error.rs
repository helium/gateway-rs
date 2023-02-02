use thiserror::Error;

pub type Result<T = ()> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("system time")]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error("protobuf decode")]
    Prost(#[from] prost::DecodeError),
    #[error("unsupported region {0}")]
    UnsupportedRegion(i32),
    #[error("no applicable region plan")]
    NoRegionParams,
    #[error("no region spreading in region plan")]
    NoRegionSpreading,
    #[error("unsupported region spreading {0}")]
    UnsupportedRegionSpreading(i32),
    #[error("no valid region spreading for packet size {0}")]
    NoRegionSpreadingAvailable(usize),
    #[error("no plausible conducted power")]
    InvalidConductedPower,
    #[error("invalid beacon version")]
    InvalidVersion,
    #[error("no valid datarate found")]
    NoDataRate,
}

impl Error {
    pub fn no_region_params() -> Self {
        Self::NoRegionParams
    }

    pub fn invalid_conducted_power() -> Self {
        Self::InvalidConductedPower
    }

    pub fn no_region_spreading() -> Self {
        Self::NoRegionSpreading
    }

    pub fn no_region_spreading_for_size(packet_size: usize) -> Self {
        Self::NoRegionSpreadingAvailable(packet_size)
    }
    pub fn unsupported_region_spreading(v: i32) -> Self {
        Self::UnsupportedRegionSpreading(v)
    }

    pub fn unsupported_region(v: i32) -> Self {
        Self::UnsupportedRegion(v)
    }

    pub fn invalid_version() -> Self {
        Self::InvalidVersion
    }

    pub fn no_data_rate() -> Self {
        Self::NoDataRate
    }
}
