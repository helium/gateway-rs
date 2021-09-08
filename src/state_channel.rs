use crate::{
    error::{StateChannelError, StateChannelSummaryError},
    router::QuePacket,
    service::gateway::GatewayService,
    Error, Keypair, MsgSign, MsgVerify, Packet, Region, Result,
};
use bytes::{Buf, BufMut, BytesMut};
use helium_crypto::PublicKey;
use helium_proto::{
    blockchain_state_channel_message_v1::Msg, BlockchainStateChannelBannerV1,
    BlockchainStateChannelMessageV1, BlockchainStateChannelOfferV1, BlockchainStateChannelPacketV1,
    BlockchainStateChannelPurchaseV1, BlockchainStateChannelRejectionV1,
    BlockchainStateChannelResponseV1, BlockchainStateChannelSummaryV1, BlockchainStateChannelV1,
    Message,
};
use sha2::{Digest, Sha256};
use std::{cmp::max, convert::TryFrom, mem};

#[derive(Debug)]
pub struct StateChannelMessage(pub(crate) Msg);

impl StateChannelMessage {
    pub fn packet(
        packet: Packet,
        keypair: &Keypair,
        region: Region,
        hold_time: u64,
    ) -> Result<Self> {
        let mut packet = BlockchainStateChannelPacketV1 {
            packet: Some(packet.to_packet()),
            signature: vec![],
            hotspot: keypair.public_key().into(),
            region: region.into(),
            hold_time,
        };
        packet.signature = packet.sign(keypair)?;
        Ok(StateChannelMessage::from(packet))
    }

    pub fn offer(packet: Packet, keypair: &Keypair, region: Region) -> Result<Self> {
        let frame = Packet::parse_frame(lorawan::Direction::Uplink, packet.payload())?;
        let mut offer = BlockchainStateChannelOfferV1 {
            packet_hash: packet.hash(),
            payload_size: packet.payload().len() as u64,
            fcnt: frame.fcnt().unwrap_or(0) as u32,
            hotspot: keypair.public_key().into(),
            region: region.into(),
            routing: Packet::routing_information(&frame)?,
            signature: vec![],
        };
        offer.signature = offer.sign(keypair)?;
        Ok(Self::from(offer))
    }

    pub fn msg(&self) -> &Msg {
        &self.0
    }

    pub fn to_message(self) -> BlockchainStateChannelMessageV1 {
        BlockchainStateChannelMessageV1 { msg: Some(self.0) }
    }

    pub fn state_channel(&self) -> Option<&BlockchainStateChannelV1> {
        match &self.0 {
            Msg::Banner(BlockchainStateChannelBannerV1 { sc }) => sc.as_ref(),
            Msg::Purchase(BlockchainStateChannelPurchaseV1 { sc, .. }) => sc.as_ref(),
            _ => None,
        }
    }

    pub fn downlink(&self) -> Option<&helium_proto::Packet> {
        match &self.0 {
            Msg::Response(BlockchainStateChannelResponseV1 { downlink, .. }) => downlink.as_ref(),
            Msg::Packet(BlockchainStateChannelPacketV1 { packet, .. }) => packet.as_ref(),
            _ => None,
        }
    }
}

impl From<Msg> for StateChannelMessage {
    fn from(v: Msg) -> Self {
        Self(v)
    }
}

macro_rules! from_msg {
    ($msg_type:ty, $enum:path) => {
        impl From<$msg_type> for StateChannelMessage {
            fn from(inner: $msg_type) -> Self {
                let msg = $enum(inner);
                Self(msg)
            }
        }

        impl From<StateChannelMessage> for $msg_type {
            fn from(v: StateChannelMessage) -> $msg_type {
                match v.0 {
                    $enum(inner) => inner,
                    _ => panic!("invalid state channel message conversion"),
                }
            }
        }
    };
}

from_msg!(BlockchainStateChannelResponseV1, Msg::Response);
from_msg!(BlockchainStateChannelPacketV1, Msg::Packet);
from_msg!(BlockchainStateChannelOfferV1, Msg::Offer);
from_msg!(BlockchainStateChannelPurchaseV1, Msg::Purchase);
from_msg!(BlockchainStateChannelBannerV1, Msg::Banner);
from_msg!(BlockchainStateChannelRejectionV1, Msg::Reject);

#[derive(PartialEq, Debug)]
pub enum StateChannelCausality {
    Effect,
    Cause,
    Equal,
    Conflict,
}

#[derive(Debug)]
pub struct StateChannel {
    sc: BlockchainStateChannelV1,
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
        if buf.len() < (mem::size_of::<u64>() * 2) {
            return Err(Error::Decode(
                prost::DecodeError::new("not enough data").into(),
            ));
        }
        let expiry_at_block = buf.get_u64();
        let original_dc_amount = buf.get_u64();
        let sc = BlockchainStateChannelV1::decode(buf)?;
        Ok(Self {
            sc,
            expiry_at_block,
            original_dc_amount,
        })
    }
}

impl StateChannel {
    pub async fn from_sc<T>(sc: T, gateway: &mut GatewayService) -> Result<Self>
    where
        T: GetStateChannel,
    {
        match sc.state_channel() {
            None => Err(StateChannelError::not_found()),
            Some(sc) => {
                let resp = gateway.is_active(&sc.id, &sc.owner).await?;
                if !resp.active {
                    return Err(StateChannelError::inactive());
                }
                Ok(Self {
                    sc,
                    expiry_at_block: resp.sc_expiry_at_block,
                    original_dc_amount: resp.sc_original_dc_amount,
                })
            }
        }
    }

    pub fn with_sc<T>(&self, sc: T) -> Result<Self>
    where
        T: GetStateChannel,
    {
        match sc.state_channel() {
            None => Err(StateChannelError::not_found()),
            Some(sc) => Ok(Self {
                sc,
                expiry_at_block: self.expiry_at_block,
                original_dc_amount: self.original_dc_amount,
            }),
        }
    }

    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut buf = BytesMut::new();
        buf.put_u64(self.expiry_at_block);
        buf.put_u64(self.original_dc_amount);
        self.sc.encode(&mut buf)?;
        Ok(buf.to_vec())
    }

    ///  Validates this state channel for just the gateway with the given public key
    ///
    /// This assumes the caller will validatea that the state channel is active.
    pub fn is_valid_sc_for(&self, public_key: &PublicKey, newer: &Self) -> Result {
        newer.is_valid_for(public_key)?;
        if self.causally_compare_for(public_key, newer) == StateChannelCausality::Conflict {
            return Err(StateChannelError::causal_conflict());
        }
        if newer.is_overpaid(self) {
            return Err(StateChannelError::overpaid());
        }
        Ok(())
    }

    pub fn is_valid_for(&self, public_key: &PublicKey) -> Result {
        PublicKey::try_from(&self.sc.owner[..])
            .map_err(|_| StateChannelError::invalid_owner())
            .and_then(|owner| self.sc.verify(&owner))
            .map_err(|_| StateChannelError::invalid_owner())?;
        // Validate summary for this gateway
        if let Some(summary) = self.get_summary(public_key) {
            self.is_valid_summary(summary)?;
        }
        Ok(())
    }

    pub fn is_valid_purchase(&self, purchase_sc: &Self, packet: Option<&QuePacket>) -> Result {
        let budget_dc = purchase_sc.amount();
        let total_dc = purchase_sc.total_dcs();
        let remaining_dc = max(0, budget_dc - total_dc);
        if self.is_overpaid(purchase_sc) {
            return Err(StateChannelError::overpaid());
        }
        if packet.is_none() {
            // The packet was not given, accept the purchase as is
            return Ok(());
        }
        let packet_dc = packet.unwrap().dc_payload();
        if remaining_dc < packet_dc {
            // Not enough remaining balance in the state channel to pay for the
            // packet
            return Err(StateChannelError::low_balance());
        }
        if (total_dc - self.total_dcs()) < packet_dc {
            // We did not get paid enough for this packet since the total_dcs in
            // the purchase did not increase at least by packet_dc
            return Err(StateChannelError::underpaid());
        }
        Ok(())
    }

    pub fn is_overpaid(&self, newer: &StateChannel) -> bool {
        self.original_dc_amount < newer.total_dcs()
    }

    pub fn causally_compare_for(
        &self,
        public_key: &PublicKey,
        newer: &Self,
    ) -> StateChannelCausality {
        match (self.sc.nonce, newer.sc.nonce) {
            (older_nonce, newer_nonce) if older_nonce == newer_nonce => {
                if self.sc.summaries == newer.sc.summaries {
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

    pub fn total_dcs(&self) -> u64 {
        self.sc
            .summaries
            .iter()
            .fold(0, |acc, summary| acc + summary.num_dcs)
    }

    pub fn get_summary(&self, public_key: &PublicKey) -> Option<&BlockchainStateChannelSummaryV1> {
        let public_keybin = public_key.to_vec();
        self.sc
            .summaries
            .iter()
            .find(|summary| summary.client_pubkeybin == public_keybin)
    }

    pub fn is_valid_summary(&self, summary: &BlockchainStateChannelSummaryV1) -> Result {
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

    pub fn id(&self) -> &[u8] {
        &self.sc.id
    }

    pub fn amount(&self) -> u64 {
        self.sc.credits
    }

    pub fn hash_key(&self) -> String {
        base64::encode_config(self.hash(), base64::URL_SAFE_NO_PAD)
    }

    pub fn hash(&self) -> Vec<u8> {
        let mut buf = vec![];
        self.sc.encode(&mut buf).expect("encoded state channel");
        Sha256::digest(&buf).to_vec()
    }
}

pub trait StateChannelKey {
    fn id_key(&self) -> String;
}

impl StateChannelKey for StateChannel {
    fn id_key(&self) -> String {
        self.sc.id.id_key()
    }
}

impl StateChannelKey for &StateChannel {
    fn id_key(&self) -> String {
        self.sc.id.id_key()
    }
}

impl StateChannelKey for Vec<u8> {
    fn id_key(&self) -> String {
        base64::encode_config(self, base64::URL_SAFE_NO_PAD)
    }
}

impl StateChannelKey for &Vec<u8> {
    fn id_key(&self) -> String {
        base64::encode_config(self, base64::URL_SAFE_NO_PAD)
    }
}

pub trait GetStateChannel {
    fn state_channel(self) -> Option<BlockchainStateChannelV1>;
}

impl GetStateChannel for BlockchainStateChannelBannerV1 {
    fn state_channel(self) -> Option<BlockchainStateChannelV1> {
        self.sc
    }
}

impl GetStateChannel for BlockchainStateChannelPurchaseV1 {
    fn state_channel(self) -> Option<BlockchainStateChannelV1> {
        self.sc
    }
}

impl GetStateChannel for BlockchainStateChannelV1 {
    fn state_channel(self) -> Option<BlockchainStateChannelV1> {
        Some(self)
    }
}
