use std::{
    collections::VecDeque,
    ops::Deref,
    time::{Duration, Instant},
};

pub trait MessageHash {
    fn hash(&self) -> Vec<u8>;
}

#[derive(Debug)]
pub struct MessageCache<T: PartialEq + MessageHash> {
    cache: VecDeque<CacheMessage<T>>,
    max_messages: u16,
}

#[derive(Debug, Clone)]
pub struct CacheMessage<T: PartialEq + MessageHash> {
    received: Instant,
    message: T,
}

impl<T: PartialEq + MessageHash> CacheMessage<T> {
    pub fn new(message: T, received: Instant) -> Self {
        Self { message, received }
    }

    pub fn hold_time(&self) -> Duration {
        self.received.elapsed()
    }
}

impl<T: PartialEq + MessageHash> Deref for CacheMessage<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

pub enum PopFront<'a> {
    Duration(Duration),
    Ack(&'a [u8]),
}

impl<T: PartialEq + MessageHash> MessageCache<T> {
    pub fn new(max_messages: u16) -> Self {
        let waiting = VecDeque::new();
        Self {
            cache: waiting,
            max_messages,
        }
    }

    /// Pushes a given at the end of the cache. The message is tagged with the
    /// given received time which can be used to calculate hold time of a
    /// packet.
    ///
    /// Pushing a packet onto the back of a full cache will cause the oldest
    /// (first) message in the cache to be dropped.
    pub fn push_back(&mut self, message: T, received: Instant) -> Option<CacheMessage<T>> {
        self.cache.push_back(CacheMessage::new(message, received));
        if self.len() > self.max_messages as usize {
            self.cache.pop_front()
        } else {
            None
        }
    }

    /// Returns the index of the first matching message in the cache or None if
    /// not present
    pub fn index_of(&self, message: &T) -> Option<usize> {
        self.cache.iter().position(|m| m.message == *message)
    }

    /// Promotes the given message to the back of the queue, effectively
    /// recreating an LRU cache. Returns true if a cache hit was found
    pub fn tag(&mut self, message: T, received: Instant) -> bool {
        let result = self
            .index_of(&message)
            .and_then(|index| self.cache.remove(index))
            .is_some();
        self.push_back(message, received);
        result
    }

    pub fn tag_now(&mut self, message: T) -> bool {
        self.tag(message, Instant::now())
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
        self.cache.push_front(cache_message);
    }

    pub fn pop_front(&mut self, args: PopFront) -> (usize, Option<CacheMessage<T>>) {
        let mut dropped = 0;
        let mut front = None;
        while let Some(msg) = self.cache.pop_front() {
            match args {
                PopFront::Duration(duration) => {
                    if msg.hold_time() <= duration {
                        front = Some(msg);
                        break;
                    }
                }
                PopFront::Ack(ack) => {
                    if msg.hash() == ack {
                        front = self.cache.pop_front();
                        break;
                    }
                }
            };
            // held for too long or acked, count as dropped and move on
            dropped += 1;
        }
        (dropped, front)
    }

    /// Returns a reference to the first (and oldest/first to be removed)
    /// message in the cache
    pub fn peek_front(&self) -> Option<&CacheMessage<T>> {
        self.cache.front()
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

#[cfg(test)]
mod test {
    use super::MessageCache;

    #[test]
    fn test_cache_tagging() {
        let mut cache = MessageCache::<Vec<u8>>::new(2);

        // First should trigger a "not in cache"
        assert!(!cache.tag_now(vec![1]));
        // Second should trigger a "not in cache" and make the first least
        // recently used
        assert!(!cache.tag_now(vec![2]));
        // Second tag should promote the old entry but remove none
        assert!(cache.tag_now(vec![1]));
        assert_eq!(
            cache.peek_front().map(|entry| entry.message.as_ref()),
            Some([2u8].as_ref())
        );

        // Third tag should evict the least recently used entry (2)
        assert!(!cache.tag_now(vec![3]));
        assert_eq!(Some(0), cache.index_of(&vec![1u8]));
        assert_eq!(Some(1), cache.index_of(&vec![3u8]));
        assert!(cache.index_of(&vec![2u8]).is_none());
    }
}
