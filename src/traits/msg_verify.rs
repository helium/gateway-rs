use crate::{Error, Result};
use helium_crypto::{PublicKey, Verify};
use helium_proto::{services::iot_config::GatewayRegionParamsResV1, Message};

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
                verifier.verify(&buf, &self.$sig).map_err(Error::from)
            }
        }
    };
}

impl_msg_verify!(GatewayRegionParamsResV1, signature);
