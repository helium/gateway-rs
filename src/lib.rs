pub mod beaconer;
pub mod cmd;
pub mod error;
pub mod gateway;
pub mod keyed_uri;
pub mod keypair;
pub mod message_cache;
pub mod packet;

pub mod packet_router;
pub mod region_watcher;
pub mod server;
pub mod service;
pub mod settings;
pub mod sync;

mod api;
mod traits;

pub use beacon::{Region, RegionParams};
pub use error::{Error, Result};
pub use keyed_uri::KeyedUri;
pub use keypair::{Keypair, PublicKey};
pub use packet::{PacketDown, PacketUp};
pub use settings::Settings;
pub(crate) use traits::*;

use futures::{Future as StdFuture, Stream as StdStream};
use std::pin::Pin;

/// A type alias for `Future` that may return `crate::error::Error`
pub type Future<T> = Pin<Box<dyn StdFuture<Output = Result<T>> + Send>>;

/// A type alias for `Stream` that may result in `crate::error::Error`
pub type Stream<T> = Pin<Box<dyn StdStream<Item = Result<T>> + Send>>;

pub async fn sign<K>(keypair: K, data: Vec<u8>) -> Result<Vec<u8>>
where
    K: AsRef<Keypair> + std::marker::Send + 'static,
{
    use futures::TryFutureExt;
    use helium_crypto::Sign;
    let join_handle: tokio::task::JoinHandle<Result<Vec<u8>>> =
        tokio::task::spawn_blocking(move || {
            keypair.as_ref().sign(&data).map_err(crate::Error::from)
        });
    join_handle
        .map_err(|err| helium_crypto::Error::from(signature::Error::from_source(err)))
        .await?
}

macro_rules! verify {
    ($key: expr, $msg: expr, $sig: ident) => {{
        let mut _msg = $msg.clone();
        _msg.$sig = vec![];
        let buf = _msg.encode_to_vec();
        $key.verify(&buf, &$msg.$sig).map_err(Error::from)
    }};
}

pub(crate) use verify;
