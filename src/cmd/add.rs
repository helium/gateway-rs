use crate::{cmd::*, keypair::Keypair, service::api, Error, PublicKey, Result, Settings};
use helium_crypto::Sign;
use helium_proto::{BlockchainTxn, BlockchainTxnAddGatewayV1, Message, Txn};
use serde_derive::Deserialize;
use serde_json::json;
use std::{fmt, str::FromStr};
use structopt::StructOpt;

/// Construct an add gateway transaction for this gateway.
#[derive(Debug, StructOpt)]
pub struct Cmd {
    /// The target owner account of this gateway
    #[structopt(long)]
    owner: PublicKey,

    /// The account that will pay account for this addition
    #[structopt(long)]
    payer: PublicKey,

    /// The staking mode for adding the light gateway
    #[structopt(long, default_value = "dataonly")]
    mode: StakingMode,
}

const TXN_FEE_SIGNATURE_SIZE: usize = 64;

impl Cmd {
    pub async fn run(&self, settings: Settings) -> Result {
        let public_key = &settings.keypair.public_key();
        let config = TxnFeeConfig::for_address(public_key).await?;
        let mut txn = BlockchainTxnAddGatewayV1 {
            gateway: public_key.to_vec(),
            owner: self.owner.to_vec(),
            payer: self.payer.to_vec(),
            fee: 0,
            staking_fee: config.get_staking_fee(&self.mode),
            owner_signature: vec![],
            gateway_signature: vec![],
            payer_signature: vec![],
        };

        txn.fee = txn_fee(&config, &txn)?;
        txn.gateway_signature = txn_sign(&settings.keypair, &txn)?;

        print_txn(&self.mode, &txn)
    }
}

fn txn_fee(config: &TxnFeeConfig, txn: &BlockchainTxnAddGatewayV1) -> Result<u64> {
    let mut txn = txn.clone();
    txn.owner_signature = vec![0; TXN_FEE_SIGNATURE_SIZE];
    txn.payer_signature = vec![0; TXN_FEE_SIGNATURE_SIZE];
    txn.gateway_signature = vec![0; TXN_FEE_SIGNATURE_SIZE];
    Ok(config.get_txn_fee(to_envelope_vec(&txn)?.len()))
}

fn txn_sign(keypair: &Keypair, txn: &BlockchainTxnAddGatewayV1) -> Result<Vec<u8>> {
    let mut txn = txn.clone();
    txn.owner_signature = vec![];
    txn.payer_signature = vec![];
    txn.gateway_signature = vec![];
    Ok(keypair.sign(&to_vec(&txn)?)?)
}

fn to_envelope_vec(txn: &BlockchainTxnAddGatewayV1) -> Result<Vec<u8>> {
    let envelope = BlockchainTxn {
        txn: Some(Txn::AddGateway(txn.clone())),
    };
    let mut buf = vec![];
    envelope.encode(&mut buf)?;
    Ok(buf)
}

fn to_vec(txn: &BlockchainTxnAddGatewayV1) -> Result<Vec<u8>> {
    let mut buf = vec![];
    txn.encode(&mut buf)?;
    Ok(buf)
}

#[derive(Debug)]
enum StakingMode {
    DataOnly,
    Light,
    Full,
}

impl fmt::Display for StakingMode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StakingMode::DataOnly => f.write_str("dataonly"),
            StakingMode::Full => f.write_str("full"),
            StakingMode::Light => f.write_str("light"),
        }
    }
}

impl FromStr for StakingMode {
    type Err = Error;
    fn from_str(v: &str) -> Result<Self> {
        match v.to_lowercase().as_ref() {
            "light" => Ok(Self::Light),
            "full" => Ok(Self::Full),
            "dataonly" => Ok(Self::DataOnly),
            _ => Err(Error::custom(format!("invalid staking mode {}", v))),
        }
    }
}

#[derive(Clone, Deserialize, Debug)]
pub struct TxnFeeConfig {
    // whether transaction fees are active
    txn_fees: bool,
    // a mutliplier which will be applied to the txn fee of all txns, in order
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

    async fn for_address(address: &PublicKey) -> Result<Self> {
        let client = api::Service::blockchain(address.network);
        let config: Self = client.get("/vars").await?;
        Ok(config)
    }

    fn get_staking_fee(&self, staking_mode: &StakingMode) -> u64 {
        match staking_mode {
            StakingMode::Full => self.staking_fee_txn_add_gateway_v1,
            StakingMode::DataOnly => self.staking_fee_txn_add_dataonly_gateway_v1,
            StakingMode::Light => self.staking_fee_txn_add_light_gateway_v1,
        }
    }

    fn get_txn_fee(&self, payload_size: usize) -> u64 {
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

fn print_txn(mode: &StakingMode, txn: &BlockchainTxnAddGatewayV1) -> Result {
    let table = json!({
        "mode": mode.to_string(),
        "address": PublicKey::from_bytes(&txn.gateway)?.to_string(),
        "payer": PublicKey::from_bytes(&txn.payer)?.to_string(),
        "owner": PublicKey::from_bytes(&txn.owner)?.to_string(),
        "fee": txn.fee,
        "staking fee": txn.staking_fee,
        "txn": base64::encode_config(&to_envelope_vec(txn)?, base64::STANDARD),
    });
    print_json(&table)
}
