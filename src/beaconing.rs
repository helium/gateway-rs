//! This module provides proof-of-coverage (PoC) beaconing support.
//!
//! TODO: where to get beacon interval from?
//!
//! TODO: what to beacon?

use crate::{gateway::MessageSender, RawPacket, Result};
use lorawan::{MType, PHYPayload, PHYPayloadFrame, MHDR};
use slog::{self, Logger};
use std::time::Duration;
use tokio::time;
use triggered::Listener;

/// How often we send a beacon.
///
/// We use this value if it's provided `Beaconer::new`.
const DEFAULT_BEACON_INTERVAL_SECS: u64 = 5 * 60;

/// Construct a proprietary LoRaWAN packet which we use our beacons.
fn make_beacon(payload: impl Into<Vec<u8>>) -> PHYPayload {
    PHYPayload {
        mhdr: {
            let mut mhdr = MHDR(0);
            mhdr.set_mtype(MType::Proprietary);
            mhdr
        },
        payload: PHYPayloadFrame::Proprietary(payload.into()),
        // TODO: get rid of MIC?
        mic: [0, 1, 2, 3],
    }
}

#[derive(Debug)]
pub struct Beaconer {
    /// gateway packet transmit message queue.
    txq: MessageSender,
    /// Beacon interval
    interval: Duration,
    /// Monotonic Beacon counter
    ctr: u32,
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
            ctr: 0,
            logger,
        }
    }

    /// Sends a gateway-to-gateway packet.
    ///
    /// See [`MessageSender::transmit_raw`]
    pub async fn send_broadcast(&mut self) -> Result {
        let payload = {
            // Packet bytes:
            // [ 'p', 'o', 'c', Ctr_byte_0, Ctr_byte_b1, Ctr_byte_b2, Ctr_byte_b3 ]
            let phy_payload_frame = b"poc"
                .iter()
                .chain(self.ctr.to_le_bytes().iter())
                .copied()
                .collect::<Vec<u8>>();
            let phy_payload = make_beacon(phy_payload_frame);
            slog::debug!(self.logger, "beacon {:?}", phy_payload);
            let mut payload = vec![];
            phy_payload.write(&mut &mut payload)?;
            self.ctr += 1;
            payload
        };
        let packet = RawPacket {
            downlink: false,
            frequency: 903_900_000,
            datarate: "SF7BW125".parse()?,
            payload,
            // Will be overridden by regional parameters
            power_dbm: 0,
        };
        slog::info!(self.logger, "sending beacon {:?}", packet);
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

#[test]
fn test_beacon_roundtrip() {
    let phy_payload_a = {
        let ctr = 12345_u32;
        // Packet bytes:
        // [ 'p', 'o', 'c', Ctr_byte_0, Ctr_byte_b1, Ctr_byte_b2, Ctr_byte_b3 ]
        let phy_payload_frame = b"poc"
            .iter()
            .chain(ctr.to_le_bytes().iter())
            .copied()
            .collect::<Vec<u8>>();
        make_beacon(phy_payload_frame)
    };
    let mut payload = vec![];
    phy_payload_a.write(&mut &mut payload).unwrap();
    let phy_payload_b = PHYPayload::read(lorawan::Direction::Uplink, &mut &payload[..]).unwrap();
    assert_eq!(phy_payload_a, phy_payload_b);
}
