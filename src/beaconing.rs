//! This module provides proof-of-coverage (PoC) beaconing support.
//!
//! TODO: where to get beacon interval from?
//!
//! TODO: what to beacon?

use crate::{gateway::MessageSender, RawPacket, Result};
use slog::{self, Logger};
use std::time::Duration;
use tokio::time;
use triggered::Listener;

/// How often we send a beacon.
///
/// We use this value if it's provided `Beaconer::new`.
const DEFAULT_BEACON_INTERVAL_SECS: u64 = 5 * 60;

#[derive(Debug)]
pub struct Beaconer {
    /// gateway packet transmit message queue.
    txq: MessageSender,
    /// Beacon interval
    interval: Duration,
    ///
    logger: Logger,
}

impl Beaconer {
    pub fn new(
        transmit_queue: MessageSender,
        beacon_interval_secs: Option<u64>,
        logger: &Logger,
    ) -> Self {
        let logger = logger.new(slog::o!("module" => "beacon"));

        let interval = match beacon_interval_secs {
            None => {
                slog::info!(
                    logger,
                    "using default beacon interval seconds: {}",
                    DEFAULT_BEACON_INTERVAL_SECS
                );
                Duration::from_secs(DEFAULT_BEACON_INTERVAL_SECS)
            }
            Some(secs) => {
                slog::info!(logger, "using provided beacon interval seconds: {}", secs);
                Duration::from_secs(secs)
            }
        };

        Self {
            txq: transmit_queue,
            interval,
            logger,
        }
    }

    /// Sends a gateway-to-gateway packet.
    ///
    /// See [`MessageSender::transmit_raw`]
    pub async fn send_broadcast(&self) -> Result {
        let packet = RawPacket {
            downlink: false,
            frequency: 903_900_000,
            datarate: "SF7BW125".parse()?,
            payload: b"hello".to_vec(),
            // Will be overridden by regional parameters
            power_dbm: 0,
        };
        self.txq.transmit_raw(packet).await?;
        Ok(())
    }

    /// Enter `Beaconer`'s run loop.
    ///
    /// This routine is will run forever and only returns on error or
    /// shut-down event (.e.g. Control-C, signal).
    pub async fn run(&mut self, shutdown: Listener) -> Result {
        slog::info!(self.logger, "starting");

        let mut intervalometer = time::interval(self.interval);

        loop {
            if shutdown.is_triggered() {
                return Ok(());
            }

            tokio::select! {
                _ = shutdown.clone() => {
                    slog::info!(self.logger, "shutting down");
                    return Ok(())
                },
                _ = intervalometer.tick() => {
                    self.send_broadcast().await?
                }
            }
        }
    }
}
