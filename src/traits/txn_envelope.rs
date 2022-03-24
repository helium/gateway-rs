use crate::{error::DecodeError, Result};
use helium_proto::{
    BlockchainTxn, BlockchainTxnAddGatewayV1, BlockchainTxnStateChannelCloseV1, Message, Txn,
};

pub trait TxnEnvelope {
    fn in_envelope(&self) -> BlockchainTxn;
    fn in_envelope_vec(&self) -> Result<Vec<u8>> {
        let envelope = self.in_envelope();
        let mut buf = vec![];
        envelope.encode(&mut buf)?;
        Ok(buf)
    }
    fn from_envelope(txn: BlockchainTxn) -> Result<Self>
    where
        Self: Sized;
    fn from_envelope_vec(buf: &[u8]) -> Result<Self>
    where
        Self: Sized;
}

macro_rules! impl_txn_envelope {
    ($txn_type: ty, $kind: ident) => {
        impl TxnEnvelope for $txn_type {
            fn in_envelope(&self) -> BlockchainTxn {
                BlockchainTxn {
                    txn: Some(Txn::$kind(self.clone())),
                }
            }

            fn from_envelope(envelope: BlockchainTxn) -> Result<Self>
            where
                Self: Sized,
            {
                match envelope.txn {
                    Some(Txn::$kind(result)) => Ok(result),
                    _ => Err(DecodeError::invalid_envelope()),
                }
            }

            fn from_envelope_vec(buf: &[u8]) -> Result<Self>
            where
                Self: Sized,
            {
                let envelope = BlockchainTxn::decode(buf)?;
                Self::from_envelope(envelope)
            }
        }
    };
}

impl_txn_envelope!(BlockchainTxnAddGatewayV1, AddGateway);
impl_txn_envelope!(BlockchainTxnStateChannelCloseV1, StateChannelClose);
