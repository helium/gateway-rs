use crate::{settings::StakingMode, Result};
use helium_proto::{BlockchainTxn, BlockchainTxnAddGatewayV1, Message, Txn};
use serde::Deserialize;

pub trait TxnFee {
    fn txn_fee(&self, config: &TxnFeeConfig) -> Result<u64>;
}

const TXN_FEE_SIGNATURE_SIZE: usize = 64;
const TXN_FEE_MULTIPLIER: u64 = 5000;

macro_rules! payer_sig_clear {
    (basic, $txn:ident) => {};
    (payer, $txn:ident) => {
        if $txn.payer.is_empty() {
            $txn.payer_signature = vec![]
        } else {
            $txn.payer_signature = vec![0; TXN_FEE_SIGNATURE_SIZE]
        };
    };
}

macro_rules! impl_txn_fee {
    (($kind:ident, $txn_type:ty, $txn_env:expr), $( $sig:ident ),+ ) => {
        impl TxnFee for $txn_type {
            fn txn_fee(&self, config: &TxnFeeConfig) -> Result<u64> {
                let mut txn: $txn_type = self.clone();
                txn.fee = 0;
                $(txn.$sig = vec![0; TXN_FEE_SIGNATURE_SIZE];)+
                payer_sig_clear!($kind, txn);
                let buf = BlockchainTxn {
                    txn: Some($txn_env(txn))
                }.encode_to_vec();
                Ok(config.get_txn_fee(buf.len()))
            }
        }
    };
    ($txn_type:ty, $($tail:tt)*) => {
        impl_txn_fee!((basic, $txn_type), $($tail)*);
    }
}

impl_txn_fee!(
    (payer, BlockchainTxnAddGatewayV1, Txn::AddGateway),
    owner_signature,
    gateway_signature
);

// TODO: Transaction fees are hard coded in the default implementation,
// specifically whether txn fees are enabled and what the dc multiplier is
// supposed to be.
#[derive(Clone, Deserialize, Debug)]
pub struct TxnFeeConfig {
    // whether transaction fees are active
    txn_fees: bool,
    // a multiplier which will be applied to the txn fee of all txns, in order
    // to make their DC costs meaningful
    txn_fee_multiplier: u64,
    // the staking fee in DC for adding a gateway
    #[serde(default = "TxnFeeConfig::default_full_staking_fee")]
    staking_fee_txn_add_gateway_v1: u64,
    // the staking fee in DC for adding a data only gateway
    #[serde(default = "TxnFeeConfig::default_dataonly_staking_fee")]
    staking_fee_txn_add_dataonly_gateway_v1: u64,
}

impl Default for TxnFeeConfig {
    fn default() -> Self {
        Self {
            txn_fees: true,
            txn_fee_multiplier: TXN_FEE_MULTIPLIER,
            staking_fee_txn_add_gateway_v1: Self::default_full_staking_fee(),
            staking_fee_txn_add_dataonly_gateway_v1: Self::default_dataonly_staking_fee(),
        }
    }
}

impl TxnFeeConfig {
    fn default_full_staking_fee() -> u64 {
        4000000
    }

    fn default_dataonly_staking_fee() -> u64 {
        1000000
    }

    pub fn get_staking_fee(&self, staking_mode: &StakingMode) -> u64 {
        match staking_mode {
            StakingMode::Full => self.staking_fee_txn_add_gateway_v1,
            StakingMode::DataOnly => self.staking_fee_txn_add_dataonly_gateway_v1,
        }
    }

    pub fn get_txn_fee(&self, payload_size: usize) -> u64 {
        let dc_payload_size = if self.txn_fees { 24 } else { 1 };
        let fee = if payload_size <= dc_payload_size {
            1
        } else {
            // integer div/ceil from: https://stackoverflow.com/a/2745086
            ((payload_size + dc_payload_size - 1) / dc_payload_size) as u64
        };
        fee * self.txn_fee_multiplier
    }
}
