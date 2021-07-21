use crate::Result;
use helium_crypto::{Keypair, Sign};
use helium_proto::{BlockchainStateChannelOfferV1, BlockchainStateChannelPacketV1, Message};

pub trait MsgSign: Message + std::clone::Clone {
    fn sign(&self, keypair: &Keypair) -> Result<Vec<u8>>
    where
        Self: std::marker::Sized;
}

macro_rules! impl_msg_sign {
    ($txn_type:ty, $( $sig: ident ),+ ) => {
        impl MsgSign for $txn_type {
            fn sign(&self, keypair: &Keypair) -> Result<Vec<u8>> {
                let mut buf = vec![];
                let mut txn = self.clone();
                $(txn.$sig = vec![];)+
                txn.encode(& mut buf)?;
                keypair.sign(&buf).map_err(|err| err.into())
            }
        }
    };
}

impl_msg_sign!(BlockchainStateChannelPacketV1, signature);
impl_msg_sign!(BlockchainStateChannelOfferV1, signature);
