use std::{
    collections::VecDeque,
    ops::Deref,
    time::{Duration, Instant},
};

pub struct MessageCache<T> {
    waiting: VecDeque<CacheMessage<T>>,
    max_messages: u16,
}

#[derive(Debug, Clone)]
pub struct CacheMessage<T> {
    received: Instant,
    message: T,
}

impl<T> CacheMessage<T> {
    pub fn new(message: T, received: Instant) -> Self {
        Self { message, received }
    }

    pub fn hold_time(&self) -> Duration {
        self.received.elapsed()
    }
}

impl<T> Deref for CacheMessage<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl<T> MessageCache<T> {
    pub fn new(max_messages: u16) -> Self {
        let waiting = VecDeque::new();
        Self {
            waiting,
            max_messages,
        }
    }

    /// Pushes a given at the end of the cache. The message is tagged with the
    /// given received time which can be used to calculate hold time of a
    /// packet.
    ///
    /// Pushing a packet onto the back of a full cache will cause the oldest
    /// (first) message in the cache to be dropped.
    pub fn push_back(&mut self, message: T, received: Instant) {
        self.waiting.push_back(CacheMessage::new(message, received));
        if self.len() > self.max_messages as usize {
            self.waiting.pop_front();
        }
    }

    /// Pushes a CacheMessage back on the front of the queue. This is useful to
    /// push a packet back at the front after a failed delivery attempt.
    ///
    /// Pushing to the front of a full cache will cause the given message to not
    /// be added.
    pub fn push_front(&mut self, cache_message: CacheMessage<T>) {
        if self.len() > self.max_messages as usize {
            return;
        }
        self.waiting.push_front(cache_message);
    }

    pub fn pop_front(&mut self, duration: Duration) -> (usize, Option<CacheMessage<T>>) {
        let mut dropped = 0;
        let mut front = None;
        while let Some(msg) = self.waiting.pop_front() {
            if msg.hold_time() <= duration {
                front = Some(msg);
                break;
            }
            // held for too long, count as dropped and move on
            dropped += 1;
        }
        (dropped, front)
    }

    pub fn len(&self) -> usize {
        self.waiting.len()
    }

    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }
}
