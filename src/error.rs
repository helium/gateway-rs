use std::net;
use thiserror::Error;

pub type Result<T = ()> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("config error: {0}")]
    Config(#[from] config::ConfigError),
    #[error("custom error: {0}")]
    Custom(String),
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
    #[error("crypto error: {0}")]
    CryptoError(#[from] helium_crypto::Error),
    #[error("encode error: {0}")]
    Encode(#[from] EncodeError),
    #[error("decode error: {0}")]
    Decode(#[from] DecodeError),
    #[error("service error: {0}")]
    Service(#[from] ServiceError),
    #[error("semtech udp error: {0}")]
    Semtech(#[from] Box<semtech_udp::server_runtime::Error>),
    #[error("{0}")]
    Beacon(#[from] beacon::Error),
    #[error("gateway error: {0}")]
    Gateway(#[from] crate::gateway::GatewayError),
    #[error("region error: {0}")]
    Region(#[from] RegionError),
    #[error("system time: {0}")]
    SystemTime(#[from] std::time::SystemTimeError),
}

#[derive(Error, Debug)]
pub enum EncodeError {
    #[error("protobuf encode")]
    Prost(#[from] prost::EncodeError),
}

#[derive(Error, Debug)]
pub enum DecodeError {
    #[error("uri decode: {0}")]
    Uri(#[from] http::uri::InvalidUri),
    #[error("keypair uri: {0}")]
    KeypairUri(String),
    #[error("json decode: {0}")]
    Json(#[from] serde_json::Error),
    #[error("base58 decode: {0}")]
    Base58(#[from] bs58::decode::Error),
    #[error("base64 decode: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("network address decode: {0}")]
    Addr(#[from] net::AddrParseError),
    #[error("protobuf decode {0}")]
    Prost(#[from] prost::DecodeError),
    #[error("lorawan decode: {0}")]
    LoraWan(#[from] lorawan::LoraWanError),
    #[error("crc is invalid and packet may be corrupted")]
    CrcInvalid,
    #[error("crc is disabled")]
    CrcDisabled,
    #[error("unexpected transaction in envelope")]
    InvalidEnvelope,
    #[error("no rx1 window in downlink packet")]
    NoRx1Window,
    #[error("packet is not a beacon")]
    NotBeacon,
    #[error("invalid datarate: {0}")]
    InvalidDataRate(String),
}

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("service {0}")]
    Service(#[from] helium_proto::services::Error),
    #[error("rpc {0}")]
    Rpc(#[from] tonic::Status),
    #[error("stream closed")]
    Stream,
    #[error("channel closed")]
    Channel,
    #[error("no active session")]
    NoSession,
    #[error("age {age}s > {max_age}s")]
    Check { age: u64, max_age: u64 },
    #[error("Unable to connect to local server. Check that `helium_gateway` is running.")]
    LocalClientConnect(helium_proto::services::Error),
}

#[derive(Debug, Error)]
pub enum RegionError {
    #[error("no region params found or active")]
    NoRegionParams,
}

macro_rules! from_err {
    ($to_type:ty, $from_type:ty) => {
        impl From<$from_type> for Error {
            fn from(v: $from_type) -> Self {
                Self::from(<$to_type>::from(v))
            }
        }
    };
}

// Service Errors
from_err!(ServiceError, helium_proto::services::Error);
from_err!(ServiceError, tonic::Status);

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for Error {
    fn from(_err: tokio::sync::mpsc::error::SendError<T>) -> Self {
        Self::Service(ServiceError::Stream)
    }
}

// Encode Errors
from_err!(EncodeError, prost::EncodeError);

// Decode Errors
from_err!(DecodeError, http::uri::InvalidUri);
from_err!(DecodeError, base64::DecodeError);
from_err!(DecodeError, bs58::decode::Error);
from_err!(DecodeError, serde_json::Error);
from_err!(DecodeError, net::AddrParseError);
from_err!(DecodeError, prost::DecodeError);
from_err!(DecodeError, lorawan::LoraWanError);

impl DecodeError {
    pub fn invalid_envelope() -> Error {
        Error::Decode(DecodeError::InvalidEnvelope)
    }

    pub fn crc_invalid() -> Error {
        Error::Decode(DecodeError::CrcInvalid)
    }

    pub fn crc_disabled() -> Error {
        Error::Decode(DecodeError::CrcInvalid)
    }

    pub fn prost_decode(msg: &'static str) -> Error {
        Error::Decode(prost::DecodeError::new(msg).into())
    }

    pub fn keypair_uri<T: ToString>(msg: T) -> Error {
        Error::Decode(DecodeError::KeypairUri(msg.to_string()))
    }

    pub fn no_rx1_window() -> Error {
        Error::Decode(DecodeError::NoRx1Window)
    }

    pub fn invalid_data_rate(datarate: String) -> Error {
        Error::Decode(DecodeError::InvalidDataRate(datarate))
    }

    pub fn not_beacon() -> Error {
        Error::Decode(DecodeError::NotBeacon)
    }
}

impl RegionError {
    pub fn no_region_params() -> Error {
        Error::Region(RegionError::NoRegionParams)
    }
}

impl Error {
    /// Use as for custom or rare errors that don't quite deserve their own
    /// error
    pub fn custom<T: ToString>(msg: T) -> Error {
        Error::Custom(msg.to_string())
    }

    pub fn channel() -> Error {
        Error::Service(ServiceError::Channel)
    }

    pub fn no_session() -> Error {
        Error::Service(ServiceError::NoSession)
    }

    pub fn no_stream() -> Error {
        Error::Service(ServiceError::Stream)
    }

    pub fn gateway_service_check(age: u64, max_age: u64) -> Error {
        Error::Service(ServiceError::Check { age, max_age })
    }

    pub fn local_client_connect(e: helium_proto::services::Error) -> Error {
        Error::Service(ServiceError::LocalClientConnect(e))
    }
}
