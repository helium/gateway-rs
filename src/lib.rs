pub mod cmd;
pub mod curl;
pub mod error;
pub mod gateway;
pub mod keyed_uri;
pub mod keypair;
pub mod link_packet;
pub mod region;
pub mod releases;
pub mod router;
pub mod server;
pub mod service;
pub mod settings;
pub mod updater;

pub use error::{Error, Result};
pub use keyed_uri::KeyedUri;
pub use keypair::{Keypair, PublicKey};
pub use link_packet::LinkPacket;
pub use region::Region;
pub use settings::Settings;

use futures::{Future as StdFuture, Stream as StdStream};
use std::pin::Pin;

/// A type alias for `Future` that may return `crate::error::Error`
pub type Future<T> = Pin<Box<dyn StdFuture<Output = Result<T>> + Send>>;

/// A type alias for `Stream` that may result in `crate::error::Error`
pub type Stream<T> = Pin<Box<dyn StdStream<Item = Result<T>> + Send>>;
