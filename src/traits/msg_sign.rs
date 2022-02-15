use crate::{Error, Result};
use futures::TryFutureExt;
use helium_crypto::{Keypair, Sign};
use helium_proto::{
    BlockchainStateChannelOfferV1, BlockchainStateChannelPacketV1, BlockchainTxnAddGatewayV1,
    BlockchainTxnStateChannelCloseV1, GatewayRegionParamsUpdateReqV1, Message,
};
use std::sync::Arc;

#[async_trait::async_trait]
pub trait MsgSign: Message + std::clone::Clone {
    async fn sign(&self, keypair: Arc<Keypair>) -> Result<Vec<u8>>
    where
        Self: std::marker::Sized;
}

macro_rules! impl_msg_sign {
    ($txn_type:ty, $( $sig: ident ),+ ) => {
        #[async_trait::async_trait]
        impl MsgSign for $txn_type {
            async fn sign(&self, keypair: Arc<Keypair>) -> Result<Vec<u8>> {
                let mut txn = self.clone();
                $(txn.$sig = vec![];)+
                let buf = txn.encode_to_vec();
                let join_handle: tokio::task::JoinHandle<Result<Vec<u8>>> = tokio::task::spawn_blocking(move ||  {
                    keypair.sign(&buf).map_err(Error::from)
                });
                join_handle.map_err(|err| helium_crypto::Error::from(signature::Error::from_source(err))).await?
            }
        }
    };
}

impl_msg_sign!(GatewayRegionParamsUpdateReqV1, signature);
impl_msg_sign!(BlockchainStateChannelPacketV1, signature);
impl_msg_sign!(BlockchainStateChannelOfferV1, signature);
impl_msg_sign!(BlockchainTxnStateChannelCloseV1, signature);
impl_msg_sign!(
    BlockchainTxnAddGatewayV1,
    owner_signature,
    payer_signature,
    gateway_signature
);
