use crate::{
    error::{DecodeError, StateChannelError, StateChannelSummaryError},
    router::{store::StateChannelEntry, QuePacket, RouterStore},
    service::gateway::GatewayService,
    Error, MsgVerify, Result,
};
use bytes::{Buf, BufMut, BytesMut};
use helium_crypto::PublicKey;
use helium_proto::{
    blockchain_state_channel_diff_entry_v1, BlockchainStateChannelDiffAppendSummaryV1,
    BlockchainStateChannelDiffUpdateSummaryV1, BlockchainStateChannelDiffV1,
    BlockchainStateChannelSummaryV1, BlockchainStateChannelV1, Message,
};
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
    pub fn is_valid_purchase_sc(
        self,
        public_key: &PublicKey,
        packet: Option<&QuePacket>,
        newer: &BlockchainStateChannelV1,
    ) -> Result<Self> {
        newer
            .is_valid_owner()
            .and_then(|_| newer.is_valid_for(public_key))?;
        let new_sc = Self {
            sc: newer.clone(),
            total_dcs: newer.total_dcs(),
            expiry_at_block: self.expiry_at_block,
            original_dc_amount: self.original_dc_amount,
        };
        let causality = (&self.sc).causally_compare_for(public_key, &newer);
        // Chheck that the purchase is an effect of the current one to avoid
        // double payment
        if causality != StateChannelCausality::Cause {
            return Err(StateChannelError::causal_conflict(self, new_sc));
        }
        self.is_valid_packet_purchase(new_sc, packet)
    }

    pub fn is_valid_purchase_sc_diff(
        self,
        _public_key: &PublicKey,
        packet: Option<&QuePacket>,
        diff: &BlockchainStateChannelDiffV1,
    ) -> Result<Self> {
        let mut new_sc = self.clone();
        new_sc.sc.nonce += diff.add_nonce;
        for diff in &diff.diffs {
            match &diff.entry {
                Some(blockchain_state_channel_diff_entry_v1::Entry::Append(
                    BlockchainStateChannelDiffAppendSummaryV1 {
                        client_pubkeybin,
                        num_packets,
                        num_dcs,
                    },
                )) => {
                    let new_summary = BlockchainStateChannelSummaryV1 {
                        client_pubkeybin: client_pubkeybin.clone(),
                        num_packets: *num_packets,
                        num_dcs: *num_dcs,
                    };
                    new_sc.sc.summaries.push(new_summary);
                    new_sc.total_dcs += num_dcs;
                }
                Some(blockchain_state_channel_diff_entry_v1::Entry::Add(
                    BlockchainStateChannelDiffUpdateSummaryV1 {
                        client_index,
                        add_packets,
                        add_dcs,
                    },
                )) => {
                    if let Some(summary) = new_sc.sc.summaries.get_mut(*client_index as usize) {
                        summary.num_packets += add_packets;
                        summary.num_dcs += add_dcs;
                        new_sc.total_dcs += add_dcs;
                    }
                }
                _ => (),
            }
        }
        self.is_valid_packet_purchase(new_sc, packet)
    }

    fn is_valid_packet_purchase(
        &self,
        new_sc: StateChannel,
        packet: Option<&QuePacket>,
    ) -> Result<StateChannel> {
        let original_dc_amount = new_sc.original_dc_amount;
        if new_sc.total_dcs > original_dc_amount {
            return Err(StateChannelError::overpaid(new_sc, original_dc_amount));
        }
        if let Some(packet) = packet {
            let dc_total = (&new_sc.sc).total_dcs();
            let dc_prev_total = (&self.sc).total_dcs();
            let dc_packet = packet.dc_payload();
            // Check that the dc change between the last state chanel and the
            // new one is at least incremented by the dcs for the packet.
            if (dc_total - dc_prev_total) < dc_packet {
                return Err(StateChannelError::underpaid(new_sc));
            }
        }
        Ok(new_sc)
    }

    pub fn id(&self) -> &[u8] {
        &self.sc.id
    }

    pub fn owner(&self) -> &[u8] {
        &self.sc.owner
    }

    pub fn amount(&self) -> u64 {
        self.sc.credits
    }

    pub fn hash(&self) -> Vec<u8> {
        let mut buf = vec![];
        self.sc.encode(&mut buf).expect("encoded state channel");
        Sha256::digest(&buf).to_vec()
    }
}

pub trait StateChannelValidation {
    fn is_valid_owner(&self) -> Result;
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

pub async fn check_active_diff(
    diff: &BlockchainStateChannelDiffV1,
    store: &RouterStore,
) -> Result<StateChannel> {
    match store.get_state_channel_entry(&diff.id) {
        None =>
        // No entry is not good for a diff since there's no state channel to
        // clone
        {
            Err(StateChannelError::not_found(&diff.id))
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
    fn is_valid_owner(&self) -> Result {
        PublicKey::try_from(&self.owner[..])
            .map_err(|_| StateChannelError::invalid_owner())
            .and_then(|owner| self.verify(&owner))
            .map_err(|_| StateChannelError::invalid_owner())?;
        Ok(())
    }

    fn is_valid_for(&self, public_key: &PublicKey) -> Result {
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
