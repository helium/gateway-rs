use crate::{error::StateChannelError, CacheSettings, Packet, Result, StateChannel};
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
    state_channel: StateChannel,
    in_conflict: bool,
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

    pub fn get_state_channel(&self, sk: &[u8]) -> Result<Option<&StateChannel>> {
        match self.state_channels.get(&sk.to_vec()) {
            None => Ok(None),
            Some(StateChannelEntry {
                in_conflict,
                state_channel,
            }) => {
                if *in_conflict {
                    Err(StateChannelError::causal_conflict())
                } else {
                    Ok(Some(state_channel))
                }
            }
        }
    }

    pub fn store_conflicting_state_channel(&mut self, sc: StateChannel) -> Result {
        self.state_channels.insert(
            sc.id().to_vec(),
            StateChannelEntry {
                in_conflict: true,
                state_channel: sc,
            },
        );
        Ok(())
    }

    pub fn store_state_channel(&mut self, sc: StateChannel) -> Result {
        self.state_channels.insert(
            sc.id().to_vec(),
            StateChannelEntry {
                in_conflict: false,
                state_channel: sc,
            },
        );
        Ok(())
    }

    pub fn state_channel_count(&self) -> usize {
        self.state_channels.len()
    }
}
