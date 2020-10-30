use std::net;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GWError {
    #[error("config error")]
    ConfigError(#[from] config::ConfigError),
    #[error("server error")]
    ServerError(String),
    #[error("io error")]
    IOError(#[from] std::io::Error),
    #[error("longfi error")]
    LfcError(#[from] longfi::LfcError),
}

impl From<net::AddrParseError> for GWError {
    fn from(v: net::AddrParseError) -> Self {
        Self::ServerError(v.to_string())
    }
}

impl From<tokio::sync::broadcast::error::RecvError> for GWError {
    fn from(v: tokio::sync::broadcast::error::RecvError) -> Self {
        Self::ServerError(v.to_string())
    }
}

impl From<openssl::error::ErrorStack> for GWError {
    fn from(v: openssl::error::ErrorStack) -> Self {
        Self::ServerError(v.to_string())
    }
}

impl From<lorawan::LoraWanError> for GWError {
    fn from(v: lorawan::LoraWanError) -> Self {
        Self::ServerError(v.to_string())
    }
}

impl From<semtech_udp::server_runtime::Error> for GWError {
    fn from(v: semtech_udp::server_runtime::Error) -> Self {
        Self::ServerError(v.to_string())
    }
}

impl From<prost::EncodeError> for GWError {
    fn from(v: prost::EncodeError) -> Self {
        Self::ServerError(v.to_string())
    }
}
