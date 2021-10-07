use crate::{state_channel::StateChannel, CacheSettings, Packet, Result};
use std::{
    collections::{HashMap, VecDeque},
    ops::Deref,
    time::{Duration, Instant},
};

pub struct RouterStore {
    state_channels: HashMap<Vec<u8>, StateChannelEntry>,
    waiting_packets: VecDeque<QuePacket>,
    queued_packets: HashMap<Vec<u8>, QuePacket>,
    max_packets: u16,
}

pub struct StateChannelEntry {
    pub(crate) ignore: bool,
    pub(crate) sc: StateChannel,
    pub(crate) conflicts_with: Option<StateChannel>,
}

impl StateChannelEntry {
    pub fn in_conflict(&self) -> bool {
        self.conflicts_with.is_some()
    }
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
}

impl Deref for QuePacket {
    type Target = Packet;

    fn deref(&self) -> &Self::Target {
        &self.packet
    }
}

impl From<Packet> for QuePacket {
    fn from(packet: Packet) -> Self {
        let received = Instant::now();
        Self { received, packet }
    }
}

impl RouterStore {
    pub fn new(settings: &CacheSettings) -> Self {
        let max_packets = settings.max_packets;
        let waiting_packets = VecDeque::new();
        let queued_packets = HashMap::new();
        let state_channels = HashMap::new();
        Self {
            waiting_packets,
            queued_packets,
            max_packets,
            state_channels,
        }
    }

    pub fn store_waiting_packet(&mut self, packet: Packet) -> Result {
        self.waiting_packets.push_back(QuePacket::from(packet));
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

    pub fn packet_queue_full(&self) -> bool {
        self.packet_queue_len() > self.max_packets as usize
    }

    pub fn packet_queue_len(&self) -> usize {
        self.queued_packets.len()
    }

    pub fn queue_packet(&mut self, packet: QuePacket) -> Result {
        self.queued_packets.insert(packet.hash(), packet);
        Ok(())
    }

    /// Removes and returns the queued packets with the given packet_hash if it
    /// exists.
    pub fn dequeue_packet(&mut self, packet_hash: &[u8]) -> Option<QuePacket> {
        self.queued_packets.remove(packet_hash)
    }

    /// Removes queued packets older than the given duration. Returns the number
    /// of packets that were removed.
    pub fn gc_queued_packets(&mut self, duration: Duration) -> usize {
        let before_len = self.queued_packets.len();
        self.queued_packets
            .retain(|_, packet| packet.received.elapsed() <= duration);
        before_len - self.queued_packets.len()
    }

    pub fn get_state_channel_entry(&self, sk: &[u8]) -> Option<&StateChannelEntry> {
        self.state_channels.get(&sk.to_vec())
    }

    pub fn get_state_channel_entry_mut(&mut self, sk: &[u8]) -> Option<&mut StateChannelEntry> {
        self.state_channels.get_mut(&sk.to_vec())
    }

    pub fn store_conflicting_state_channel(
        &mut self,
        sc: StateChannel,
        conflicts_with: StateChannel,
    ) -> Result {
        self.state_channels.insert(
            sc.id().to_vec(),
            StateChannelEntry {
                ignore: true,
                sc,
                conflicts_with: Some(conflicts_with),
            },
        );
        Ok(())
    }

    pub fn ignore_state_channel(&mut self, sc: StateChannel) -> Result {
        self.state_channels.insert(
            sc.id().to_vec(),
            StateChannelEntry {
                ignore: true,
                sc,
                conflicts_with: None,
            },
        );
        Ok(())
    }

    pub fn store_state_channel(&mut self, sc: StateChannel) -> Result {
        self.state_channels.insert(
            sc.id().to_vec(),
            StateChannelEntry {
                ignore: false,
                sc,
                conflicts_with: None,
            },
        );
        Ok(())
    }

    pub fn remove_state_channel(&mut self, sk: &[u8]) -> Option<StateChannelEntry> {
        self.state_channels.remove(&sk.to_vec())
    }

    pub fn state_channel_count(&self) -> usize {
        self.state_channels.len()
    }
}
