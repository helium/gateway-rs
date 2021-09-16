use crate::{
    error::{Error, StateChannelError},
    CacheSettings, Packet, Result, StateChannel, StateChannelKey,
};
use std::{
    collections::VecDeque,
    convert::TryFrom,
    io,
    ops::Deref,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::fs;

pub struct RouterStore {
    path: PathBuf,
    waiting_packets: VecDeque<QuePacket>,
    queued_packets: VecDeque<QuePacket>,
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
    pub async fn new(name: &str, settings: &CacheSettings) -> Result<Self> {
        let path = settings.store.join(name);
        fs::create_dir_all(&path).await?;
        clean_dir(&path).await?;
        let max_packets = settings.max_packets;
        let waiting_packets = VecDeque::new();
        let queued_packets = VecDeque::new();
        Ok(Self {
            path,
            waiting_packets,
            queued_packets,
            max_packets,
        })
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

    pub async fn state_channel_count(&self) -> Result<usize> {
        Ok(file_names(&self.path).await?.len())
    }

    pub async fn get_state_channel<S>(&self, sk: S) -> Result<Option<StateChannel>>
    where
        S: StateChannelKey,
    {
        let sc_id = sk.id_key();
        let hashes = self.get_state_channel_hashes(&sc_id).await?;
        if hashes.is_empty() {
            return Ok(None);
        } else if hashes.len() > 1 {
            return Err(StateChannelError::causal_conflict());
        }
        let data = fs::read(self.path.join(sc_id).join(&hashes[0])).await?;
        Ok(Some(StateChannel::try_from(&data[..])?))
    }

    pub async fn append_state_channel(&self, sc_id: &str, sc: &StateChannel) -> Result {
        let sc_hash = sc.hash_key();
        let known_hashes = self.get_state_channel_hashes(sc_id).await?;
        // Only add if we don't already have it to save writing multiple times
        if known_hashes.contains(&sc_hash) {
            return Ok(());
        }
        let file_path = self.path.join(sc_id).join(sc_hash);
        fs::write(file_path, sc.to_vec()?)
            .await
            .map_err(Error::from)
    }

    pub async fn overwrite_state_channel(&self, sc_id: &str, sc: &StateChannel) -> Result {
        let sc_path = self.path.join(sc_id);
        clean_dir(&sc_path).await?;
        let sc_hash = sc.hash_key();
        fs::write(&sc_path.join(sc_hash), sc.to_vec()?)
            .await
            .map_err(Error::from)
    }

    async fn get_state_channel_hashes(&self, sc_id: &str) -> Result<Vec<String>> {
        match file_names(self.path.join(sc_id)).await {
            Ok(names) => Ok(names),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(vec![]),
            Err(other) => Err(other.into()),
        }
    }
}

async fn clean_dir<P: AsRef<Path>>(path: P) -> io::Result<()> {
    fs::create_dir_all(&path).await?;
    let mut entries = fs::read_dir(path).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if entry.file_type().await?.is_dir() {
            fs::remove_dir_all(path).await?;
        } else {
            fs::remove_file(path).await?;
        }
    }
    Ok(())
}

async fn file_names<P: AsRef<Path>>(path: P) -> io::Result<Vec<String>> {
    use futures::StreamExt;
    use tokio_stream::wrappers::ReadDirStream;
    let entries = ReadDirStream::new(fs::read_dir(path).await?);
    let names = entries
        .filter_map(|entry| async move {
            match entry {
                Ok(entry) => entry.file_name().into_string().ok(),
                Err(err) => panic!("filesystem error: {:?}", err),
            }
        })
        .collect()
        .await;
    Ok(names)
}
