use thiserror::Error;

pub type Result<T = ()> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("system time")]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error("no applicable region plan")]
    NoRegionParams,
    #[error("invalid beacon version")]
    InvalidVersion,
}

impl Error {
    pub fn no_region_params() -> Self {
        Self::NoRegionParams
    }

    pub fn invalid_version() -> Self {
        Self::InvalidVersion
    }
}
