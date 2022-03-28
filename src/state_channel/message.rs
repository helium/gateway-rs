use crate::{Keypair, MsgSign, Packet, Region, Result};
use helium_proto::{
    blockchain_state_channel_message_v1::Msg, BlockchainStateChannelMessageV1,
    BlockchainStateChannelOfferV1, BlockchainStateChannelPacketV1,
};
use std::sync::Arc;

#[derive(Debug)]
pub struct StateChannelMessage(pub(crate) Msg);

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
        Ok(StateChannelMessage::from(packet))
    }

    pub async fn offer(
        packet: Packet,
        keypair: Arc<Keypair>,
        region: Region,
        req_diff: bool,
    ) -> Result<Self> {
        let frame = Packet::parse_frame(lorawan::Direction::Uplink, packet.payload())?;
        let mut offer = BlockchainStateChannelOfferV1 {
            packet_hash: packet.hash(),
            payload_size: packet.payload().len() as u64,
            fcnt: frame.fcnt().unwrap_or(0) as u32,
            hotspot: keypair.public_key().into(),
            region: region.into(),
            routing: Packet::routing_information(&frame)?,
            signature: vec![],
            req_diff,
        };
        offer.signature = offer.sign(keypair).await?;
        Ok(Self::from(offer))
    }

    pub fn msg(&self) -> &Msg {
        &self.0
    }

    pub fn to_message(self) -> BlockchainStateChannelMessageV1 {
        BlockchainStateChannelMessageV1 { msg: Some(self.0) }
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

from_msg!(BlockchainStateChannelPacketV1, Msg::Packet);
from_msg!(BlockchainStateChannelOfferV1, Msg::Offer);
