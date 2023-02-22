use crate::{Keypair, Result};

#[async_trait::async_trait]
pub trait MsgSign: helium_proto::Message + std::clone::Clone {
    async fn sign<T>(&self, keypair: T) -> Result<Vec<u8>>
    where
        Self: std::marker::Sized,
        T: AsRef<Keypair> + std::marker::Send + 'static;
}

macro_rules! impl_msg_sign {
    ($txn_type:ty, $( $sig: ident ),+ ) => {
        #[async_trait::async_trait]
        impl MsgSign for $txn_type {
            async fn sign<T>(&self, keypair: T) -> Result<Vec<u8>>
            where T: AsRef<Keypair> + std::marker::Send + 'static {
                use helium_proto::Message;
                use futures::TryFutureExt;
                use helium_crypto::Sign;
                let mut txn = self.clone();
                $(txn.$sig = vec![];)+
                let buf = txn.encode_to_vec();
                let join_handle: tokio::task::JoinHandle<Result<Vec<u8>>> = tokio::task::spawn_blocking(move ||  {
                    keypair.as_ref().sign(&buf).map_err(crate::Error::from)
                });
                join_handle.map_err(|err| helium_crypto::Error::from(signature::Error::from_source(err))).await?
            }
        }
    };
}

pub(crate) use impl_msg_sign;
