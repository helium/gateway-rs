use crate::{
    api::LocalClient, cmd::*, settings::StakingMode, Base64, PublicKey, Result, Settings,
    TxnEnvelope,
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
        let mut client = LocalClient::new(settings.api).await?;

        let txn = client
            .add_gateway(&self.owner, &self.payer, &self.mode)
            .await?;
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
        "txn": txn.in_envelope_vec()?.to_b64(),
    });
    print_json(&table)
}
