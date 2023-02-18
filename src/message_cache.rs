use crate::Result;
use std::{
    collections::VecDeque,
    ops::Deref,
    time::{Duration, Instant},
};

pub struct MessageCache<T> {
    waiting: VecDeque<CacheMessage<T>>,
    max_messages: u16,
}

#[derive(Debug)]
pub struct CacheMessage<T> {
    received: Instant,
    message: T,
}

impl<T> CacheMessage<T> {
    pub fn hold_time(&self) -> Duration {
        self.received.elapsed()
    }

    pub fn into_inner(self) -> T {
        self.message
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

    pub fn store(&mut self, message: T, received: Instant) -> Result {
        self.waiting.push_back(CacheMessage { message, received });
        if self.waiting_count() > self.max_messages as usize {
            self.waiting.pop_front();
        }
        Ok(())
    }

    pub fn pop_waiting(&mut self) -> Option<CacheMessage<T>> {
        self.waiting.pop_front()
    }

    pub fn waiting_count(&self) -> usize {
        self.waiting.len()
    }

    /// Removes waiting packets older than the given duration. Returns the number
    /// of packets that were removed.
    pub fn gc(&mut self, duration: Duration) -> usize {
        let before_len = self.waiting.len();
        self.waiting
            .retain(|msg| msg.received.elapsed() <= duration);
        before_len - self.waiting.len()
    }
}
