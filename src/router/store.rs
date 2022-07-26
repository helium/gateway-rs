use crate::{CacheSettings, Packet, Result};
use std::{
    collections::VecDeque,
    ops::Deref,
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
        Self {
            waiting_packets,
            max_packets,
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

    /// Removes waiting packets older than the given duration. Returns the number
    /// of packets that were removed.
    pub fn gc_waiting_packets(&mut self, duration: Duration) -> usize {
        let before_len = self.waiting_packets.len();
        self.waiting_packets
            .retain(|packet| packet.received.elapsed() <= duration);
        before_len - self.waiting_packets.len()
    }
}
