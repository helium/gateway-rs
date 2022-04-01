use crate::state_channel;
use std::net;
use thiserror::Error;

pub type Result<T = ()> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("config error")]
    Config(#[from] config::ConfigError),
    #[error("custom error")]
    Custom(String),
    #[error("io error")]
    IO(#[from] std::io::Error),
    #[error("crypto error")]
    CryptoError(#[from] helium_crypto::Error),
    #[error("encode error")]
    Encode(#[from] EncodeError),
    #[error("decode error")]
    Decode(#[from] DecodeError),
    #[error("service error: {0}")]
    Service(#[from] ServiceError),
    #[error("state channel error")]
    StateChannel(#[from] Box<StateChannelError>),
    #[error("semtech udp error")]
    Semtech(#[from] semtech_udp::server_runtime::Error),
    #[error("time error")]
    Time(#[from] std::time::SystemTimeError),
}

#[derive(Error, Debug)]
pub enum EncodeError {
    #[error("protobuf encode")]
    Prost(#[from] prost::EncodeError),
}

#[derive(Error, Debug)]
pub enum DecodeError {
    #[error("uri decode")]
    Uri(#[from] http::uri::InvalidUri),
    #[error("keypair uri: {0}")]
    KeypairUri(String),
    #[error("json decode")]
    Json(#[from] serde_json::Error),
    #[error("base64 decode")]
    Base64(#[from] base64::DecodeError),
    #[error("network address decode")]
    Addr(#[from] net::AddrParseError),
    #[error("protobuf decode")]
    Prost(#[from] prost::DecodeError),
    #[error("lorawan decode")]
    LoraWan(#[from] lorawan::LoraWanError),
    #[error("longfi error")]
    LfcError(#[from] longfi::LfcError),
    #[error("semtech decode")]
    Semtech(#[from] semtech_udp::data_rate::ParseError),
    #[error("packet crc")]
    InvalidCrc,
    #[error("unexpected transaction in envelope")]
    InvalidEnvelope,
}

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("service {0:?}")]
    Service(#[from] helium_proto::services::Error),
    #[error("rpc {0:?}")]
    Rpc(#[from] tonic::Status),
    #[error("stream closed")]
    Stream,
    #[error("channel closed")]
    Channel,
    #[error("no service")]
    NoService,
    #[error("block age {block_age}s > {max_age}s")]
    Check { block_age: u64, max_age: u64 },
    #[error("Unable to connect to local server. Check that `helium_gateway` is running.")]
    LocalClientConnect(helium_proto::services::Error),
}

#[allow(clippy::large_enum_variant)]
#[derive(Error, Debug)]
pub enum StateChannelError {
    #[error("ignored state channel")]
    Ignored { sc: state_channel::StateChannel },
    #[error("inactive state channel")]
    Inactive,
    #[error("state channel not found")]
    NotFound { sc_id: Vec<u8> },
    #[error("invalid owner for state channel")]
    InvalidOwner,
    #[error("state channel summary error")]
    Summary(#[from] StateChannelSummaryError),
    #[error("new state channel error")]
    NewChannel { sc: state_channel::StateChannel },
    #[error("state channel causal conflict")]
    CausalConflict {
        sc: state_channel::StateChannel,
        conflicts_with: state_channel::StateChannel,
    },
    #[error("state channel overpaid")]
    Overpaid {
        sc: state_channel::StateChannel,
        original_dc_amount: u64,
    },
    #[error("state channel underpaid for a packet")]
    Underpaid { sc: state_channel::StateChannel },
    #[error("state channel balance too low")]
    LowBalance,
}

#[derive(Error, Debug)]
pub enum StateChannelSummaryError {
    #[error("zero state channel packet summary")]
    ZeroPacket,
    #[error("zero state channel packet over dc count")]
    PacketDCMismatch,
    #[error("invalid address")]
    InvalidAddress,
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
from_err!(DecodeError, serde_json::Error);
from_err!(DecodeError, net::AddrParseError);
from_err!(DecodeError, prost::DecodeError);
from_err!(DecodeError, lorawan::LoraWanError);
from_err!(DecodeError, longfi::LfcError);
from_err!(DecodeError, semtech_udp::data_rate::ParseError);

impl DecodeError {
    pub fn invalid_envelope() -> Error {
        Error::Decode(DecodeError::InvalidEnvelope)
    }

    pub fn invalid_crc() -> Error {
        Error::Decode(DecodeError::InvalidCrc)
    }

    pub fn prost_decode(msg: &'static str) -> Error {
        Error::Decode(prost::DecodeError::new(msg).into())
    }

    pub fn keypair_uri<T: ToString>(msg: T) -> Error {
        Error::Decode(DecodeError::KeypairUri(msg.to_string()))
    }
}

// State Channel Errors
impl StateChannelError {
    pub fn invalid_owner() -> Error {
        Error::StateChannel(Box::new(Self::InvalidOwner))
    }

    pub fn invalid_summary(err: StateChannelSummaryError) -> Error {
        Error::StateChannel(Box::new(Self::Summary(err)))
    }

    pub fn inactive() -> Error {
        Error::StateChannel(Box::new(Self::Inactive))
    }

    pub fn not_found(sc_id: &[u8]) -> Error {
        let sc_id = sc_id.to_vec();
        Error::StateChannel(Box::new(Self::NotFound { sc_id }))
    }

    pub fn ignored(sc: state_channel::StateChannel) -> Error {
        Error::StateChannel(Box::new(Self::Ignored { sc }))
    }

    pub fn new_channel(sc: state_channel::StateChannel) -> Error {
        Error::StateChannel(Box::new(Self::NewChannel { sc }))
    }

    pub fn causal_conflict(
        sc: state_channel::StateChannel,
        conflicts_with: state_channel::StateChannel,
    ) -> Error {
        Error::StateChannel(Box::new(Self::CausalConflict { sc, conflicts_with }))
    }

    pub fn overpaid(sc: state_channel::StateChannel, original_dc_amount: u64) -> Error {
        Error::StateChannel(Box::new(Self::Overpaid {
            sc,
            original_dc_amount,
        }))
    }

    pub fn underpaid(sc: state_channel::StateChannel) -> Error {
        Error::StateChannel(Box::new(Self::Underpaid { sc }))
    }

    pub fn low_balance() -> Error {
        Error::StateChannel(Box::new(Self::LowBalance))
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

    pub fn no_service() -> Error {
        Error::Service(ServiceError::NoService)
    }

    pub fn local_client_connect(e: helium_proto::services::Error) -> Error {
        Error::Service(ServiceError::LocalClientConnect(e))
    }

    pub fn gateway_service_check(block_age: u64, max_age: u64) -> Error {
        Error::Service(ServiceError::Check { block_age, max_age })
    }
}
