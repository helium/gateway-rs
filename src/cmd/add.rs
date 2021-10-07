use crate::{
    cmd::*, settings::StakingMode, MsgSign, PublicKey, Result, Settings, TxnEnvelope, TxnFee,
    TxnFeeConfig,
};
use helium_proto::BlockchainTxnAddGatewayV1;
use serde_json::json;
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

        txn.fee = txn.txn_fee(&config)?;
        txn.gateway_signature = txn.sign(&settings.keypair)?;

        print_txn(&self.mode, &txn)
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
        "txn": base64::encode_config(&txn.in_envelope_vec()?, base64::STANDARD),
    });
    print_json(&table)
}
