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

    pub fn push_back(&mut self, message: T, received: Instant) {
        self.waiting.push_back(CacheMessage { message, received });
        if self.len() > self.max_messages as usize {
            self.waiting.pop_front();
        }
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
