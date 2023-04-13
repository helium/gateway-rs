use crate::{api::LocalClient, cmd::*, settings::StakingMode, Base64, PublicKey, Result, Settings};
use helium_proto::{BlockchainTxn, BlockchainTxnAddGatewayV1, Message, Txn};
use serde_json::json;

/// Construct an add gateway transaction for this gateway.
#[derive(Debug, clap::Args)]
pub struct Cmd {
    /// The target owner account of this gateway
    #[arg(long)]
    owner: PublicKey,

    /// The account that will pay account for this addition
    #[arg(long)]
    payer: PublicKey,

    /// The staking mode for adding the gateway
    #[arg(long, default_value = "dataonly")]
    mode: StakingMode,
}

impl Cmd {
    pub async fn run(&self, settings: Settings) -> Result {
        let mut client = LocalClient::new(&settings.api).await?;

        let txn = client
            .add_gateway(&self.owner, &self.payer, &self.mode)
            .await?;
        print_txn(&self.mode, txn)
    }
}

fn print_txn(mode: &StakingMode, txn: BlockchainTxnAddGatewayV1) -> Result {
    let table = json!({
        "mode": mode.to_string(),
        "address": PublicKey::from_bytes(&txn.gateway)?.to_string(),
        "payer": PublicKey::from_bytes(&txn.payer)?.to_string(),
        "owner": PublicKey::from_bytes(&txn.owner)?.to_string(),
        "fee": txn.fee,
        "staking fee": txn.staking_fee,
        "txn": BlockchainTxn {
            txn: Some(Txn::AddGateway(txn))
        }.encode_to_vec().to_b64()
    });
    print_json(&table)
}
