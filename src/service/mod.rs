use tokio::time::{self, Duration, Instant};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const RPC_TIMEOUT: Duration = Duration::from_secs(5);

pub const RECONNECT_BACKOFF_RETRIES: u32 = 40;
pub const RECONNECT_BACKOFF_MIN_WAIT: Duration = Duration::from_secs(5);
pub const RECONNECT_BACKOFF_MAX_WAIT: Duration = Duration::from_secs(1800); // 30 minutes

pub mod conduit;
pub mod config;
pub mod entropy;
pub mod packet_router;
pub mod poc;

#[derive(Debug)]
pub struct Reconnect {
    backoff: exponential_backoff::Backoff,
    next_time: Instant,
    pub max_wait: Duration,
    pub max_retries: u32,
    pub retry_count: u32,
}

impl Default for Reconnect {
    fn default() -> Self {
        Self::new(
            RECONNECT_BACKOFF_RETRIES,
            RECONNECT_BACKOFF_MIN_WAIT,
            RECONNECT_BACKOFF_MAX_WAIT,
        )
    }
}

impl Reconnect {
    pub fn new(retries: u32, min: Duration, max: Duration) -> Self {
        Self {
            backoff: exponential_backoff::Backoff::new(retries, min, max),
            next_time: Instant::now() + min,
            max_retries: retries,
            max_wait: max,
            retry_count: 0,
        }
    }

    pub fn wait(&self) -> time::Sleep {
        time::sleep_until(self.next_time)
    }

    pub fn update_next_time(&mut self, inc_retry: bool) {
        if inc_retry {
            if self.retry_count == self.max_retries {
                self.retry_count = 0;
            } else {
                self.retry_count += 1;
            }
        }
        let backoff = self.backoff.next(self.retry_count).unwrap_or(self.max_wait);
        self.next_time = Instant::now() + backoff;
    }
}
