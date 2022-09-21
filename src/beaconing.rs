//! This module provides proof-of-coverage (PoC) beaconing support.
//!
//! TODO: where to get beacon interval from?
//!
//! TODO: what to beacon?
//!
//! TODO: fuzz beacon interval to prevent thundering herd.

use crate::{
    gateway, service::poc::PocLoraService, settings::Settings, Packet, RawPacket, RegionParams,
    Result,
};
use helium_proto::{services::poc_lora, DataRate};
use lorawan::{MType, PHYPayload, PHYPayloadFrame, MHDR};
use rand::{rngs::StdRng, seq::SliceRandom, SeedableRng};
use slog::{self, debug, error, info, warn, Logger};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::{sync::mpsc, time};
use triggered::Listener;

/// How often we send a beacon.
///
/// We use this value if it's provided `Beaconer::new`.
///
/// Make sure we get a change to beacon 3x daily with a few seconds to
/// spare.
const DEFAULT_BEACON_INTERVAL_SECS: u64 = 8 * (3600 - 1);

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
    poc_service: PocLoraService,
    rng: StdRng,
    logger: Logger,
}

impl Beaconer {
    pub fn new(
        settings: &Settings,
        transmit_queue: gateway::MessageSender,
        receiver: MessageReceiver,
        logger: &Logger,
    ) -> Self {
        let logger = logger.new(slog::o!("module" => "beacon"));

        let interval = match settings.poc.beacon_interval {
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

        let poc_service = PocLoraService::new(settings.poc.ingest_uri.clone());

        Self {
            txq: transmit_queue,
            inbox: receiver,
            interval,
            ctr: 0,
            region_params: None,
            poc_service,
            rng: StdRng::from_entropy(),
            logger,
        }
    }

    // Randomly choose a frequency from our current regional
    // parameters, if any.
    fn rand_freq(&mut self) -> u64 {
        if let Some(RegionParams { params, .. }) = &self.region_params {
            params
                .as_slice()
                .choose(&mut self.rng)
                .map(|params| params.channel_frequency)
                .unwrap_or_else(|| {
                    warn!(
                        self.logger,
                        "TODO: empty channel plan for reagion, using hardcoded freq"
                    );
                    903_900_000
                })
        } else {
            warn!(
                self.logger,
                "TODO: no regional parameters, using hardcoded freq"
            );
            903_900_000
        }
    }

    /// Sends a gateway-to-gateway packet.
    ///
    /// See [`gateway::MessageSender::transmit_raw`]
    pub async fn send_broadcast(&mut self) -> Result {
        let lora_frame = {
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
        let frequency = self.rand_freq();
        let beacon_report = poc_lora::LoraBeaconReportReqV1 {
            pub_key: vec![],
            local_entropy: vec![],
            remote_entropy: vec![],
            data: lora_frame.clone(),
            frequency: frequency as u32,
            channel: 0,
            datarate: DataRate::Sf7bw125 as i32,
            tx_power: 27,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            signature: vec![],
        };
        let packet = RawPacket {
            downlink: false,
            frequency,
            datarate: "SF7BW125".parse()?,
            payload: lora_frame,
            // Will be overridden by regional parameters
            power_dbm: 0,
        };
        info!(self.logger, "sending beacon {:?}", packet);
        self.txq.transmit_raw(packet).await?;

        match self.poc_service.submit_beacon(beacon_report.clone()).await {
            Ok(resp) => info!(
                self.logger,
                "poc beacon submitted: {:?}, response: {}", beacon_report, resp
            ),
            Err(e) => warn!(
                self.logger,
                "poc beacon submitted: {:?}, err: {}", beacon_report, e
            ),
        }

        Ok(())
    }

    async fn handle_message(&mut self, message: Message) {
        match message {
            Message::RxPk(packet) => self.handle_packet(packet).await,
            Message::RegionParamsChanged(region_params) => self.handle_region_params(region_params),
        }
    }

    async fn handle_packet(&mut self, packet: Packet) {
        if let Ok(lorawan::PHYPayloadFrame::Proprietary(proprietary_payload)) =
            Packet::parse_frame(lorawan::Direction::Uplink, packet.payload())
        {
            info!(
                self.logger,
                "received possible-PoC proprietary lorawan frame {:?}", packet
            );
            let dr = match packet.datarate.as_str() {
                "SF7BW125" => DataRate::Sf7bw125,
                "SF8BW125" => DataRate::Sf8bw125,
                "SF9BW125" => DataRate::Sf9bw125,
                "SF10BW125" => DataRate::Sf10bw125,
                "SF12BW125" => DataRate::Sf12bw125,
                &_ => {
                    warn!(self.logger, "unknown datarate {}", packet.datarate);
                    return;
                }
            };
            let witness_report = poc_lora::LoraWitnessReportReqV1 {
                pub_key: vec![],
                data: proprietary_payload,
                timestamp: packet.timestamp,
                ts_res: 0,
                signal: 0,
                snr: packet.snr,
                frequency: packet.frequency,
                datarate: dr as i32,
                signature: vec![],
            };
            match self
                .poc_service
                .submit_witness(witness_report.clone())
                .await
            {
                Ok(resp) => info!(
                    self.logger,
                    "poc witness submitted: {:?}, response: {}", witness_report, resp
                ),
                Err(e) => warn!(
                    self.logger,
                    "poc witness submitted: {:?}, err: {}", witness_report, e
                ),
            }
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
