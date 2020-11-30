pub mod cmd;
pub mod error;
pub mod gateway;
pub mod keypair;
pub mod releases;
pub mod router;
pub mod server;
pub mod settings;
pub mod updater;

use crate::error::Result;
use futures::{Future as StdFuture, Stream as StdStream};
use std::pin::Pin;

/// A type alias for `Future` that may return `crate::error::Error`
pub type Future<T> = Pin<Box<dyn StdFuture<Output = Result<T>> + Send>>;

/// A type alias for `Stream` that may result in `crate::error::Error`
pub type Stream<T> = Pin<Box<dyn StdStream<Item = Result<T>> + Send>>;
