use crate::{
    api::{ConfigValue, LocalClient},
    settings::StakingMode,
    Error, Result, TxnEnvelope,
};
use helium_proto::{BlockchainTxnAddGatewayV1, BlockchainTxnStateChannelCloseV1, Message};
use serde_derive::Deserialize;

pub trait TxnFee {
    fn txn_fee(&self, config: &TxnFeeConfig) -> Result<u64>;
}

const TXN_FEE_SIGNATURE_SIZE: usize = 64;
const TXN_FEE_MULTIPLIER: u64 = 5000;
pub const CONFIG_FEE_KEYS: &[&str] = &[
    "txn_fees",
    "txn_fee_multiplier",
    "staking_fee_txn_add_gateway_v1",
    "staking_fee_txn_add_light_gateway_v1",
    "staking_fee_txn_add_dataonly_gateway_v1",
];

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
    (($kind:ident, $txn_type:ty), $( $sig:ident ),+ ) => {
        impl TxnFee for $txn_type {
            fn txn_fee(&self, config: &TxnFeeConfig) -> Result<u64> {
                let mut txn: $txn_type = self.clone();
                txn.fee = 0;
                $(txn.$sig = vec![0; TXN_FEE_SIGNATURE_SIZE];)+
                payer_sig_clear!($kind, txn);
                let mut buf = vec![];
                txn.in_envelope().encode(&mut buf)?;
                Ok(config.get_txn_fee(buf.len()))
            }
        }
    };
    ($txn_type:ty, $($tail:tt)*) => {
        impl_txn_fee!((basic, $txn_type), $($tail)*);
    }
}

impl_txn_fee!(BlockchainTxnStateChannelCloseV1, signature);
impl_txn_fee!(
    (payer, BlockchainTxnAddGatewayV1),
    owner_signature,
    gateway_signature
);

// TODO: Transaction fees are hard coded the default implementation,
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
    // the staking fee in DC for adding a light gateway
    #[serde(default = "TxnFeeConfig::default_light_staking_fee")]
    staking_fee_txn_add_light_gateway_v1: u64,
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
            staking_fee_txn_add_light_gateway_v1: Self::default_light_staking_fee(),
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

    fn default_light_staking_fee() -> u64 {
        4000000
    }

    pub async fn from_client(client: &mut LocalClient) -> Result<Self> {
        let values = client.config(CONFIG_FEE_KEYS).await?;
        Self::try_from(values)
    }

    pub fn get_staking_fee(&self, staking_mode: &StakingMode) -> u64 {
        match staking_mode {
            StakingMode::Full => self.staking_fee_txn_add_gateway_v1,
            StakingMode::DataOnly => self.staking_fee_txn_add_dataonly_gateway_v1,
            StakingMode::Light => self.staking_fee_txn_add_light_gateway_v1,
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

impl TryFrom<Vec<ConfigValue>> for TxnFeeConfig {
    type Error = Error;

    fn try_from(v: Vec<ConfigValue>) -> Result<Self> {
        let mut result = Self::default();
        for var in v.iter() {
            match var.name.as_ref() {
                "txn_fees" => result.txn_fees = var.to_value()?,
                "txn_fee_multiplier" => result.txn_fee_multiplier = var.to_value()?,
                "staking_fee_txn_add_gateway_v1" => {
                    result.staking_fee_txn_add_gateway_v1 = var.to_value()?
                }
                "staking_fee_txn_add_light_gateway_v1" => {
                    result.staking_fee_txn_add_light_gateway_v1 = var.to_value()?
                }
                "staking_fee_txn_add_dataonly_gateway_v1" => {
                    result.staking_fee_txn_add_dataonly_gateway_v1 = var.to_value()?
                }
                _ => (),
            }
        }
        Ok(result)
    }
}

trait ToValue<T> {
    fn to_value(&self) -> Result<T>;
}

impl ToValue<bool> for ConfigValue {
    fn to_value(&self) -> Result<bool> {
        let name = &self.name;
        if self.r#type != "atom" {
            return Err(Error::custom(format!("not a boolean variable: {name}",)));
        }
        let value = std::str::from_utf8(&self.value)
            .map_err(|_| Error::custom(format!("not a boolean value: {name}")))?;
        Ok(value == "true")
    }
}

impl ToValue<u64> for ConfigValue {
    fn to_value(&self) -> Result<u64> {
        let name = &self.name;
        if self.r#type != "int" {
            return Err(Error::custom(format!("not an int variable: {name}")));
        }
        let value = std::str::from_utf8(&self.value)
            .map_err(|_| Error::custom(format!("not an valid value: {name}")))
            .and_then(|v| {
                v.parse::<u64>()
                    .map_err(|_| Error::custom(format!("not a valid int value: {name}")))
            })?;
        Ok(value)
    }
}
