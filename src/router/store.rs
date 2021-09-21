use crate::{state_channel::StateChannel, CacheSettings, Packet, Result};
use std::{
    collections::{HashMap, VecDeque},
    ops::Deref,
    time::{Duration, Instant},
};

pub struct RouterStore {
    state_channels: HashMap<Vec<u8>, StateChannelEntry>,
    waiting_packets: VecDeque<QuePacket>,
    queued_packets: VecDeque<QuePacket>,
    max_packets: u16,
}

pub struct StateChannelEntry {
    pub(crate) sc: StateChannel,
    pub(crate) conflicts_with: Option<StateChannel>,
}

impl StateChannelEntry {
    pub fn set_conflicting_state_channel(&mut self, conflicts_with: StateChannel) {
        self.conflicts_with = Some(conflicts_with);
    }

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
        let queued_packets = VecDeque::new();
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
        if self.waiting_packets.len() > self.max_packets as usize {
            self.waiting_packets.pop_front();
        }
        Ok(())
    }

    pub fn pop_waiting_packet(&mut self) -> Option<QuePacket> {
        self.waiting_packets.pop_front()
    }

    pub fn que_packet(&mut self, packet: QuePacket) -> Result {
        self.queued_packets.push_back(packet);
        if self.queued_packets.len() > self.max_packets as usize {
            self.queued_packets.pop_front();
        }
        Ok(())
    }

    pub fn deque_packet(&mut self) -> Option<QuePacket> {
        self.queued_packets.pop_front()
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
                sc,
                conflicts_with: Some(conflicts_with),
            },
        );
        Ok(())
    }

    pub fn store_state_channel(&mut self, sc: StateChannel) -> Result {
        self.state_channels
            .entry(sc.id().to_vec())
            .or_insert_with(|| StateChannelEntry {
                sc,
                conflicts_with: None,
            });
        Ok(())
    }

    pub fn remove_state_channel(&mut self, sk: &[u8]) -> Option<StateChannelEntry> {
        self.state_channels.remove(&sk.to_vec())
    }

    pub fn state_channel_count(&self) -> usize {
        self.state_channels.len()
    }
}
