use crate::*;
use helium_crypto::Verify;
use helium_proto::Message;
use std::sync::Arc;

pub const CONNECT_TIMEOUT: u64 = 10;

pub mod gateway;
pub mod router;

pub trait SignatureAccess {
    fn set_signature(&mut self, signature: Vec<u8>);
    fn get_signature(&self) -> &Vec<u8>;
}

pub struct Streaming<S> {
    streaming: tonic::codec::Streaming<S>,
    verifier: Arc<PublicKey>,
}

impl<T: SignatureAccess + Message + Clone + Sync + Send> Streaming<T> {
    pub async fn message(&mut self) -> Result<Option<T>> {
        match self.streaming.message().await {
            Ok(Some(response)) => {
                // Create a clone with an empty signature
                let mut v = response.clone();
                v.set_signature(vec![]);
                // Encode the clone
                let mut buf = vec![];
                v.encode(&mut buf)?;
                // And verify against signature in the message
                self.verifier.verify(&buf, response.get_signature())?;
                Ok(Some(response))
            }
            Ok(None) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}
