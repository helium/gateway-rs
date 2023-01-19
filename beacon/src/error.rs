use thiserror::Error;

pub type Result<T = ()> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("system time")]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error("unsupported region {0}")]
    UnsupportedRegion(i32),
    #[error("no applicable region plan")]
    NoRegionParams,
    #[error("invalid beacon version")]
    InvalidVersion,
    #[error("no valid datarate found")]
    NoDataRate,
}

impl Error {
    pub fn no_region_params() -> Self {
        Self::NoRegionParams
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
