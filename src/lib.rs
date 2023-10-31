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
mod base64;

pub(crate) use crate::base64::Base64;
pub use beacon::{Region, RegionParams};
pub use error::{DecodeError, Error, Result};
pub use keyed_uri::KeyedUri;
pub use keypair::{Keypair, PublicKey, Sign, Verify};
pub use packet::{PacketDown, PacketUp};
pub use settings::Settings;

use futures::{Future as StdFuture, Stream as StdStream};
use std::pin::Pin;

/// A type alias for `Future` that may return `crate::error::Error`
pub type Future<T> = Pin<Box<dyn StdFuture<Output = Result<T>> + Send>>;

/// A type alias for `Stream` that may result in `crate::error::Error`
pub type Stream<T> = Pin<Box<dyn StdStream<Item = Result<T>> + Send>>;

async fn sign<K>(keypair: K, data: Vec<u8>) -> Result<Vec<u8>>
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

macro_rules! impl_sign {
    ($type: ty) => {
        #[tonic::async_trait]
        impl Sign for $type {
            async fn sign<K>(&mut self, keypair: K) -> Result
            where
                K: AsRef<Keypair> + std::marker::Send + 'static,
            {
                self.signature = crate::sign(keypair, self.encode_to_vec()).await?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_sign;

macro_rules! impl_verify {
    ($type: ty) => {
        impl crate::Verify for $type {
            fn verify(&self, pub_key: &crate::PublicKey) -> Result {
                use helium_crypto::Verify as _;
                let mut _msg = self.clone();
                _msg.signature = vec![];
                let buf = _msg.encode_to_vec();
                pub_key
                    .verify(&buf, &self.signature)
                    .map_err(crate::Error::from)
            }
        }
    };
}
pub(crate) use impl_verify;
