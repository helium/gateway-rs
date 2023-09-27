use crate::{api::LocalClient, cmd::*, settings::StakingMode, Base64, PublicKey, Result, Settings};
use helium_proto::{BlockchainTxn, BlockchainTxnAddGatewayV1, Message, Txn};
use serde_json::json;

/// Construct an add gateway transaction for this gateway.
#[derive(Debug, clap::Args)]
pub struct Cmd {
    /// The solana address of the target owner for this gateway
    #[arg(long, value_parser = parse_pubkey)]
    owner: PublicKey,

    /// The solana address of the payer account that will pay account for this
    /// addition
    #[arg(long, value_parser = parse_pubkey)]
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
        "payer": PublicKey::from_bytes(&txn.payer).and_then(solana_pubkey)?,
        "owner": PublicKey::from_bytes(&txn.owner).and_then(solana_pubkey)?,
        "txn": BlockchainTxn {
            txn: Some(Txn::AddGateway(txn))
        }.encode_to_vec().to_b64()
    });
    print_json(&table)
}

fn parse_pubkey(str: &str) -> Result<PublicKey> {
    use helium_crypto::{ed25519, ReadFrom};
    use std::{io::Cursor, str::FromStr};

    match PublicKey::from_str(str) {
        Ok(pk) => Ok(pk),
        Err(_) => {
            let bytes = bs58::decode(str).into_vec()?;
            let public_key = ed25519::PublicKey::read_from(&mut Cursor::new(bytes))?;
            Ok(public_key.into())
        }
    }
}

fn solana_pubkey(key: PublicKey) -> std::result::Result<String, helium_crypto::Error> {
    let bytes = &key.to_vec()[1..];
    Ok(bs58::encode(bytes).into_string())
}
