use crate::Result;
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
}

macro_rules! impl_txn_envelope {
    ($txn_type: ty, $kind: ident) => {
        impl TxnEnvelope for $txn_type {
            fn in_envelope(&self) -> BlockchainTxn {
                BlockchainTxn {
                    txn: Some(Txn::$kind(self.clone())),
                }
            }
        }
    };
}

impl_txn_envelope!(BlockchainTxnAddGatewayV1, AddGateway);
impl_txn_envelope!(BlockchainTxnStateChannelCloseV1, StateChannelClose);
