pub mod cmd;
pub mod curl;
pub mod error;
pub mod gateway;
pub mod keyed_uri;
pub mod keypair;
pub mod packet;
pub mod region;
pub mod releases;
pub mod router;
pub mod server;
pub mod service;
pub mod settings;
pub mod state_channel;
pub mod updater;

mod msg_sign;
mod msg_verify;
mod txn_envelope;
mod txn_fee;

pub use error::{Error, Result};
pub use keyed_uri::KeyedUri;
pub use keypair::{Keypair, PublicKey};
pub use msg_sign::MsgSign;
pub use msg_verify::MsgVerify;
pub use packet::Packet;
pub use region::Region;
pub use settings::{CacheSettings, Settings};
pub use txn_envelope::TxnEnvelope;
pub use txn_fee::{TxnFee, TxnFeeConfig};

use futures::{Future as StdFuture, Stream as StdStream};
use std::pin::Pin;

/// A type alias for `Future` that may return `crate::error::Error`
pub type Future<T> = Pin<Box<dyn StdFuture<Output = Result<T>> + Send>>;

/// A type alias for `Stream` that may result in `crate::error::Error`
pub type Stream<T> = Pin<Box<dyn StdStream<Item = Result<T>> + Send>>;

/// Convert a slice of bytes to a base64 url encoded string
pub fn hash_str(hash: &[u8]) -> String {
    base64::encode_config(hash, base64::URL_SAFE_NO_PAD)
}
