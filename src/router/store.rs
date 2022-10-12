use helium_proto::{services::router::PacketRouterPacketUpV1, DataRate};

use crate::{CacheSettings, Keypair, MsgSign, Packet, Region, Result};
use std::{
    collections::VecDeque,
    ops::Deref,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct RouterStore {
    waiting_packets: VecDeque<QuePacket>,
    max_packets: u16,
}

#[derive(Debug)]
pub struct QuePacket {
    received: Instant,
    packet: Packet,
}

impl QuePacket {
    pub fn hold_time(&self) -> Duration {
        self.received.elapsed()
    }

    pub fn packet(&self) -> &Packet {
        &self.packet
    }

    pub async fn to_uplink(
        &self,
        keypair: Arc<Keypair>,
        region: &Region,
    ) -> Result<PacketRouterPacketUpV1> {
        let packet = self.packet();

        let mut up = PacketRouterPacketUpV1 {
            payload: packet.payload.clone(),
            timestamp: packet.timestamp,
            rssi: packet.signal_strength as i32,
            frequency: (packet.frequency * 1_000_000.0) as u32,
            datarate: DataRate::from_str(&packet.datarate)? as i32,
            snr: packet.snr,
            region: region.into(),
            hold_time: self.hold_time().as_millis() as u64,
            gateway: keypair.public_key().into(),
            signature: vec![],
        };
        up.signature = up.sign(keypair.clone()).await?;

        Ok(up)
    }
}

impl Deref for QuePacket {
    type Target = Packet;

    fn deref(&self) -> &Self::Target {
        &self.packet
    }
}

impl RouterStore {
    pub fn new(settings: &CacheSettings) -> Self {
        let max_packets = settings.max_packets;
        let waiting_packets = VecDeque::new();
        Self {
            waiting_packets,
            max_packets,
        }
    }

    pub fn store_waiting_packet(&mut self, packet: Packet, received: Instant) -> Result {
        self.waiting_packets
            .push_back(QuePacket { packet, received });
        if self.waiting_packets_len() > self.max_packets as usize {
            self.waiting_packets.pop_front();
        }
        Ok(())
    }

    pub fn pop_waiting_packet(&mut self) -> Option<QuePacket> {
        self.waiting_packets.pop_front()
    }

    pub fn waiting_packets_len(&self) -> usize {
        self.waiting_packets.len()
    }

    /// Removes waiting packets older than the given duration. Returns the number
    /// of packets that were removed.
    pub fn gc_waiting_packets(&mut self, duration: Duration) -> usize {
        let before_len = self.waiting_packets.len();
        self.waiting_packets
            .retain(|packet| packet.received.elapsed() <= duration);
        before_len - self.waiting_packets.len()
    }
}
