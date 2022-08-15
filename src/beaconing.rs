//! This module provides proof-of-coverage (PoC) beaconing support.
//!
//! TODO: where to get beacon interval from?
//!
//! TODO: what to beacon?

use crate::{gateway, Packet, RawPacket, RegionParams, Result};
use lorawan::{MType, PHYPayload, PHYPayloadFrame, MHDR};
use slog::{self, debug, error, info, warn, Logger};
use std::time::Duration;
use tokio::{sync::mpsc, time};
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

/// Message types that can be sent to `Beaconer`'s inbox.
#[derive(Debug)]
pub enum Message {
    RxPk(Packet),
    RegionParamsChanged(RegionParams),
}

pub type MessageSender = mpsc::Sender<Message>;
pub type MessageReceiver = mpsc::Receiver<Message>;

pub fn message_channel(size: usize) -> (MessageSender, MessageReceiver) {
    mpsc::channel(size)
}

#[derive(Debug)]
pub struct Beaconer {
    /// gateway packet transmit message queue
    txq: gateway::MessageSender,
    /// Our receive queue.
    inbox: MessageReceiver,
    /// Beacon interval
    interval: Duration,
    /// Monotonic Beacon counter
    ctr: u32,
    /// Use for channel plan and FR parameters
    region_params: Option<RegionParams>,
    ///
    logger: Logger,
}

impl Beaconer {
    pub fn new(
        transmit_queue: gateway::MessageSender,
        receiver: MessageReceiver,
        beacon_interval_secs: Option<u64>,
        logger: &Logger,
    ) -> Self {
        let logger = logger.new(slog::o!("module" => "beacon"));

        let interval = match beacon_interval_secs {
            None => {
                info!(
                    logger,
                    "using default beacon interval seconds: {}", DEFAULT_BEACON_INTERVAL_SECS
                );
                Duration::from_secs(DEFAULT_BEACON_INTERVAL_SECS)
            }
            Some(secs) => {
                info!(logger, "using provided beacon interval seconds: {}", secs);
                Duration::from_secs(secs)
            }
        };

        Self {
            txq: transmit_queue,
            inbox: receiver,
            interval,
            ctr: 0,
            region_params: None,
            logger,
        }
    }

    /// Sends a gateway-to-gateway packet.
    ///
    /// See [`MessageSender::transmit_raw`]
    pub async fn send_broadcast(&mut self) -> Result {
        let region_params = if let Some(region_params) = &self.region_params {
            region_params
        } else {
            warn!(self.logger, "ignoring beacon transmit, no region params");
            return Ok(());
        };

        let payload = {
            // Packet bytes:
            // [ 'p', 'o', 'c', Ctr_byte_0, Ctr_byte_b1, Ctr_byte_b2, Ctr_byte_b3 ]
            let phy_payload_frame = b"poc"
                .iter()
                .chain(self.ctr.to_le_bytes().iter())
                .copied()
                .collect::<Vec<u8>>();
            let phy_payload = make_beacon(phy_payload_frame);
            debug!(self.logger, "beacon {:?}", phy_payload);
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
        info!(self.logger, "sending beacon {:?}", packet);
        self.txq.transmit_raw(packet).await?;

        Ok(())
    }

    async fn handle_message(&mut self, message: Message) {
        match message {
            Message::RxPk(packet) => self.handle_packet(packet),
            Message::RegionParamsChanged(region_params) => self.handle_region_params(region_params),
        }
    }

    fn handle_packet(&mut self, packet: Packet) {
        if let Ok(lorawan::PHYPayloadFrame::Proprietary(proprietary_payload)) =
            Packet::parse_frame(lorawan::Direction::Uplink, packet.payload())
        {
            info!(
                self.logger,
                "received possible-PoC proprietary lorawan frame {:?}", proprietary_payload
            );
        }
    }

    fn handle_region_params(&mut self, params: RegionParams) {
        self.region_params = Some(params);
        info!(self.logger, "updated region";
              "region" => RegionParams::to_string(&self.region_params));
    }

    /// Enter `Beaconer`'s run loop.
    ///
    /// This routine is will run forever and only returns on error or
    /// shut-down event (.e.g. Control-C, signal).
    pub async fn run(&mut self, shutdown: Listener) -> Result {
        info!(self.logger, "starting");

        let mut intervalometer = time::interval(self.interval);

        loop {
            if shutdown.is_triggered() {
                return Ok(());
            }

            tokio::select! {
                _ = shutdown.clone() => {
                    info!(self.logger, "shutting down");
                    return Ok(())
                },
                _ = intervalometer.tick() => {
                    self.send_broadcast().await?
                },
                message = self.inbox.recv() => match message {
                    Some(message) => self.handle_message(message).await,
                    None => {
                        // This state should trigger a shutdown or
                        // rebuilding of the application state, but is
                        // extremely unlikely to happen.
                        error!(self.logger, "all senders closed, I can no longer receive");
                        continue;
                    }
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
