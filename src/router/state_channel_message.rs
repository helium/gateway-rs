use crate::{impl_msg_sign, Error, Keypair, MsgSign, Packet, Region, Result};
use helium_proto::{
    blockchain_state_channel_message_v1::Msg, BlockchainStateChannelMessageV1,
    BlockchainStateChannelPacketV1,
};
use std::sync::Arc;

#[derive(Debug)]
pub struct StateChannelMessage(pub(crate) Msg);

impl_msg_sign!(BlockchainStateChannelPacketV1, signature);

impl StateChannelMessage {
    pub async fn packet(
        packet: Packet,
        keypair: Arc<Keypair>,
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
        packet.signature = packet.sign(keypair).await?;
        Ok(Self::from(packet))
    }

    pub fn msg(&self) -> &Msg {
        &self.0
    }

    pub fn to_message(self) -> BlockchainStateChannelMessageV1 {
        BlockchainStateChannelMessageV1 { msg: Some(self.0) }
    }

    pub fn to_downlink(self) -> Result<Option<Packet>> {
        match self.0 {
            Msg::Response(response) => Ok(response.downlink.map(Packet::from)),
            _ => Err(Error::custom("state channel message not a downlink packet")),
        }
    }

    pub fn from_message(msg: BlockchainStateChannelMessageV1) -> Option<Self> {
        msg.msg.map(Self::from)
    }
}

impl From<Msg> for StateChannelMessage {
    fn from(v: Msg) -> Self {
        Self(v)
    }
}

impl From<BlockchainStateChannelPacketV1> for StateChannelMessage {
    fn from(inner: BlockchainStateChannelPacketV1) -> Self {
        let msg = Msg::Packet(inner);
        Self(msg)
    }
}

impl From<StateChannelMessage> for BlockchainStateChannelPacketV1 {
    fn from(v: StateChannelMessage) -> Self {
        match v.0 {
            Msg::Packet(inner) => inner,
            _ => panic!("invalid state channel message conversion"),
        }
    }
}
