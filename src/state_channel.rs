use crate::{
    error::{StateChannelError, StateChannelSummaryError},
    service::gateway::Service as GatewayService,
    Error, MsgVerify, Result,
};
use helium_crypto::PublicKey;
use helium_proto::{
    blockchain_state_channel_message_v1::Msg, BlockchainStateChannelBannerV1,
    BlockchainStateChannelMessageV1, BlockchainStateChannelOfferV1, BlockchainStateChannelPacketV1,
    BlockchainStateChannelSummaryV1, BlockchainStateChannelV1,
};
use std::convert::TryFrom;

pub struct StateChannelMessage(BlockchainStateChannelMessageV1);

impl StateChannelMessage {}

impl From<BlockchainStateChannelOfferV1> for StateChannelMessage {
    fn from(offer: BlockchainStateChannelOfferV1) -> Self {
        let msg = BlockchainStateChannelMessageV1 {
            msg: Some(Msg::Offer(offer)),
        };
        Self(msg)
    }
}

impl From<BlockchainStateChannelPacketV1> for StateChannelMessage {
    fn from(packet: BlockchainStateChannelPacketV1) -> Self {
        let msg = BlockchainStateChannelMessageV1 {
            msg: Some(Msg::Packet(packet)),
        };
        Self(msg)
    }
}

impl From<StateChannelMessage> for BlockchainStateChannelMessageV1 {
    fn from(v: StateChannelMessage) -> Self {
        v.0
    }
}

pub struct StateChannel(BlockchainStateChannelV1);

impl From<StateChannel> for BlockchainStateChannelV1 {
    fn from(v: StateChannel) -> Self {
        v.0
    }
}

impl TryFrom<BlockchainStateChannelBannerV1> for StateChannel {
    type Error = Error;

    fn try_from(v: BlockchainStateChannelBannerV1) -> Result<Self> {
        match v {
            BlockchainStateChannelBannerV1 { sc: Some(sc) } => Ok(Self(sc)),
            _ => Err(StateChannelError::not_found()),
        }
    }
}

impl StateChannel {
    pub async fn is_valid(
        &self,
        gateway_client: &mut GatewayService,
        public_key: &PublicKey,
    ) -> Result {
        // Ensure this state channel is active
        if !gateway_client.is_active(&self.0.id, &self.0.owner).await? {
            return Err(StateChannelError::inactive());
        }
        // Validate owner
        PublicKey::try_from(&self.0.owner[..])
            .map_err(|_| StateChannelError::invalid_owner())
            .and_then(|owner| self.0.verify(&owner))
            .map_err(|_| StateChannelError::invalid_owner())?;
        // Validate summary for this gateway
        if let Some(summary) = self.get_summary(public_key) {
            self.is_valid_summary(summary)?;
        }
        // TODO: Check causality, overspend
        Ok(())
    }

    pub fn get_summary(&self, public_key: &PublicKey) -> Option<&BlockchainStateChannelSummaryV1> {
        let public_keybin = public_key.to_vec();
        self.0
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
}
