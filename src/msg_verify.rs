use crate::{Error, Result};
use helium_crypto::{PublicKey, Verify};
use helium_proto::{
    BlockchainStateChannelMessageV1, BlockchainStateChannelOfferV1, BlockchainStateChannelPacketV1,
    BlockchainStateChannelV1, GatewayRespV1, Message,
};

pub trait MsgVerify {
    fn verify(&self, verifier: &PublicKey) -> Result;
}

macro_rules! impl_msg_verify {
    ($msg_type:ty, $sig: ident) => {
        impl MsgVerify for $msg_type {
            fn verify(&self, verifier: &PublicKey) -> Result {
                let mut buf = vec![];
                let mut msg = self.clone();
                msg.$sig = vec![];
                msg.encode(&mut buf)?;
                verifier.verify(&buf, &self.$sig).map_err(|err| err.into())
            }
        }
    };
}

impl_msg_verify!(GatewayRespV1, signature);
impl_msg_verify!(BlockchainStateChannelPacketV1, signature);
impl_msg_verify!(BlockchainStateChannelOfferV1, signature);
impl_msg_verify!(BlockchainStateChannelV1, signature);

impl MsgVerify for BlockchainStateChannelMessageV1 {
    fn verify(&self, verifier: &PublicKey) -> Result {
        use helium_proto::blockchain_state_channel_message_v1::Msg;
        match &self.msg {
            Some(Msg::Response(_m)) => Ok(()),
            Some(Msg::Packet(m)) => m.verify(verifier),
            Some(Msg::Offer(m)) => m.verify(verifier),
            Some(Msg::Purchase(_m)) => Ok(()),
            Some(Msg::Banner(_m)) => Ok(()),
            Some(Msg::Reject(_m)) => Ok(()),
            None => Err(Error::custom("unexpected empty state channel message")),
        }
    }
}
