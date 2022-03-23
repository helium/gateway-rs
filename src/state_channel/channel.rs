use crate::{
    error::{DecodeError, StateChannelError, StateChannelSummaryError},
    hash_str,
    router::{store::StateChannelEntry, RouterStore},
    service::gateway::GatewayService,
    Error, MsgVerify, Result,
};
use bytes::{Buf, BufMut, BytesMut};
use helium_crypto::PublicKey;
use helium_proto::{BlockchainStateChannelSummaryV1, BlockchainStateChannelV1, Message};
use sha2::{Digest, Sha256};
use std::{convert::TryFrom, mem};

#[derive(PartialEq, Debug)]
pub enum StateChannelCausality {
    Effect,
    Cause,
    Equal,
    Conflict,
}

#[derive(Debug, Clone)]
pub struct StateChannel {
    pub(crate) sc: BlockchainStateChannelV1,
    total_dcs: u64,
    expiry_at_block: u64,
    original_dc_amount: u64,
}

impl From<StateChannel> for BlockchainStateChannelV1 {
    fn from(v: StateChannel) -> Self {
        v.sc
    }
}

impl TryFrom<&[u8]> for StateChannel {
    type Error = Error;

    fn try_from(v: &[u8]) -> Result<Self> {
        let mut buf = v;
        if buf.len() < (mem::size_of::<u64>() * 3) {
            return Err(DecodeError::prost_decode("not enough data"));
        }
        let expiry_at_block = buf.get_u64();
        let original_dc_amount = buf.get_u64();
        let total_dcs = buf.get_u64();
        let sc = BlockchainStateChannelV1::decode(buf)?;
        Ok(Self {
            sc,
            total_dcs,
            expiry_at_block,
            original_dc_amount,
        })
    }
}

impl StateChannel {
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut buf = BytesMut::new();
        buf.put_u64(self.expiry_at_block);
        buf.put_u64(self.original_dc_amount);
        buf.put_u64(self.total_dcs);
        self.sc.encode(&mut buf)?;
        Ok(buf.to_vec())
    }

    ///  Validates this state channel for just the gateway with the given public key
    ///
    /// This assumes the caller will validate that the state channel is active.
    pub fn is_valid_upgrade_for(
        self,
        public_key: &PublicKey,
        newer: &BlockchainStateChannelV1,
    ) -> Result<(Self, Self, StateChannelCausality)> {
        newer.is_valid_for(public_key)?;
        let newer_sc = Self {
            sc: newer.clone(),
            total_dcs: newer.total_dcs(),
            expiry_at_block: self.expiry_at_block,
            original_dc_amount: self.original_dc_amount,
        };
        let causality = (&self.sc).causally_compare_for(public_key, &newer);
        if causality == StateChannelCausality::Conflict {
            return Err(StateChannelError::causal_conflict(self, newer_sc));
        }
        if newer_sc.total_dcs > self.original_dc_amount {
            return Err(StateChannelError::overpaid(
                newer_sc,
                self.original_dc_amount,
            ));
        }
        Ok((self, newer_sc, causality))
    }

    pub fn id(&self) -> &[u8] {
        &self.sc.id
    }

    pub fn owner(&self) -> &[u8] {
        &self.sc.owner
    }

    pub fn id_str(&self) -> String {
        hash_str(&self.sc.id)
    }

    pub fn amount(&self) -> u64 {
        self.sc.credits
    }

    pub fn hash_str(&self) -> String {
        hash_str(&self.hash())
    }

    pub fn hash(&self) -> Vec<u8> {
        let mut buf = vec![];
        self.sc.encode(&mut buf).expect("encoded state channel");
        Sha256::digest(&buf).to_vec()
    }
}

pub trait StateChannelValidation {
    fn is_valid_for(&self, public_key: &PublicKey) -> Result;
    fn total_dcs(&self) -> u64;
    fn num_dcs_for(&self, public_key: &PublicKey) -> u64;
    fn get_summary(&self, public_key: &PublicKey) -> Option<&BlockchainStateChannelSummaryV1>;
    fn causally_compare_for(&self, public_key: &PublicKey, newer: &Self) -> StateChannelCausality;
}

pub async fn check_active(
    channel: &BlockchainStateChannelV1,
    gateway: &mut GatewayService,
    store: &RouterStore,
) -> Result<StateChannel> {
    match store.get_state_channel_entry(&channel.id) {
        None => {
            let resp = gateway.is_active_sc(&channel.id, &channel.owner).await?;
            if !resp.active {
                return Err(StateChannelError::inactive());
            }
            let new_sc = StateChannel {
                sc: channel.clone(),
                total_dcs: channel.total_dcs(),
                expiry_at_block: resp.sc_expiry_at_block,
                original_dc_amount: resp.sc_original_dc_amount,
            };
            Err(StateChannelError::new_channel(new_sc))
        }
        Some(entry) => match entry {
            // If the entry is ignored return an error
            StateChannelEntry {
                ignore: true, sc, ..
            } => Err(StateChannelError::ignored(sc.clone())),
            // Next is the conflict check
            StateChannelEntry {
                sc,
                conflicts_with: Some(conflicts_with),
                ..
            } => Err(StateChannelError::causal_conflict(
                sc.clone(),
                conflicts_with.clone(),
            )),
            // After which we're ok for a active check
            StateChannelEntry { sc, .. } => Ok(sc.clone()),
        },
    }
}

impl StateChannelValidation for &BlockchainStateChannelV1 {
    fn is_valid_for(&self, public_key: &PublicKey) -> Result {
        PublicKey::try_from(&self.owner[..])
            .map_err(|_| StateChannelError::invalid_owner())
            .and_then(|owner| self.verify(&owner))
            .map_err(|_| StateChannelError::invalid_owner())?;
        // Validate summary for this gateway
        if let Some(summary) = self.get_summary(public_key) {
            is_valid_summary(summary)?;
        }
        Ok(())
    }

    fn get_summary(&self, public_key: &PublicKey) -> Option<&BlockchainStateChannelSummaryV1> {
        let public_keybin = public_key.to_vec();
        self.summaries
            .iter()
            .find(|summary| summary.client_pubkeybin == public_keybin)
    }

    fn total_dcs(&self) -> u64 {
        self.summaries
            .iter()
            .fold(0, |acc, summary| acc + summary.num_dcs)
    }

    fn num_dcs_for(&self, public_key: &PublicKey) -> u64 {
        let public_keybin = public_key.to_vec();
        self.summaries.iter().fold(0, |acc, summary| {
            if summary.client_pubkeybin == public_keybin {
                acc + summary.num_dcs
            } else {
                acc
            }
        })
    }

    fn causally_compare_for(&self, public_key: &PublicKey, newer: &Self) -> StateChannelCausality {
        match (self.nonce, newer.nonce) {
            (older_nonce, newer_nonce) if older_nonce == newer_nonce => {
                if self.summaries == newer.summaries {
                    return StateChannelCausality::Equal;
                }
                StateChannelCausality::Conflict
            }
            (older_nonce, newer_nonce) if newer_nonce > older_nonce => {
                match (self.get_summary(public_key), newer.get_summary(public_key)) {
                    (None, _) => StateChannelCausality::Cause,
                    (Some(_), None) => StateChannelCausality::Conflict,
                    (Some(older_summary), Some(newer_summary)) => {
                        if newer_summary.num_packets >= older_summary.num_packets
                            && newer_summary.num_dcs >= older_summary.num_dcs
                        {
                            StateChannelCausality::Cause
                        } else {
                            StateChannelCausality::Conflict
                        }
                    }
                }
            }
            (_older_nonce, _newer_nonce) => {
                match (self.get_summary(public_key), newer.get_summary(public_key)) {
                    (_, None) => StateChannelCausality::Effect,
                    (None, _) => StateChannelCausality::Conflict,
                    (Some(older_summary), Some(newer_summary)) => {
                        if newer_summary.num_packets <= older_summary.num_packets
                            && newer_summary.num_dcs <= older_summary.num_packets
                        {
                            StateChannelCausality::Effect
                        } else {
                            StateChannelCausality::Conflict
                        }
                    }
                }
            }
        }
    }
}

fn is_valid_summary(summary: &BlockchainStateChannelSummaryV1) -> Result {
    PublicKey::try_from(&summary.client_pubkeybin[..]).map_err(|_| {
        StateChannelError::invalid_summary(StateChannelSummaryError::InvalidAddress)
    })?;
    if summary.num_dcs < summary.num_packets {
        return Err(StateChannelError::invalid_summary(
            StateChannelSummaryError::PacketDCMismatch,
        ));
    }
    if summary.num_packets == 0 {
        return Err(StateChannelError::invalid_summary(
            StateChannelSummaryError::ZeroPacket,
        ));
    }
    Ok(())
}
