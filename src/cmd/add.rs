use crate::{
    api::LocalClient, cmd::*, settings::StakingMode, PublicKey, Result, Settings, TxnEnvelope,
    TxnFee, TxnFeeConfig,
};
use helium_proto::{BlockchainTxnAddGatewayV1, Message};
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
        let mut client = LocalClient::new(settings.api.clone()).await?;
        let public_key = client.pubkey().await?;
        let config = TxnFeeConfig::from_client(&mut client).await?;
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
        txn.gateway_signature = client.sign(&txn.encode_to_vec()).await?;

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
