//! This module provides proof-of-coverage (PoC) beaconing support.

use crate::{
    error::RegionError,
    gateway::{self, BeaconResp},
    service::{entropy::EntropyService, poc::PocLoraService},
    settings::Settings,
    sync, Base64, Error, Keypair, MsgSign, Packet, RegionParams, Result,
};
use futures::TryFutureExt;
use helium_proto::{services::poc_lora, Message as ProtoMessage};
use http::Uri;
use rand::{rngs::OsRng, Rng};
use slog::{self, info, warn, Logger};
use std::{sync::Arc, time::Duration};
use tokio::time;
use triggered::Listener;
use xxhash_rust::xxh64::xxh64;

/// To prevent a thundering herd of hotspots all beaconing at the same
/// time, we add a randomized jitter value of up to
/// `BEACON_INTERVAL_JITTER_PERCENTAGE` to the configured beacon
/// interval. This jitter factor is one time only, and will only
/// change when this process or task restarts.
const BEACON_INTERVAL_JITTER_PERCENTAGE: u64 = 10;

/// Message types that can be sent to `Beaconer`'s inbox.
#[derive(Debug)]
pub enum Message {
    ReceivedBeacon(Packet),
    RegionParamsChanged(RegionParams),
}

pub type MessageSender = sync::MessageSender<Message>;
pub type MessageReceiver = sync::MessageReceiver<Message>;

pub fn message_channel(size: usize) -> (MessageSender, MessageReceiver) {
    sync::message_channel(size)
}

impl MessageSender {
    pub async fn received_beacon(&self, packet: Packet) {
        let _ = self
            .0
            .send(Message::ReceivedBeacon(packet))
            .map_err(|_| Error::channel())
            .await;
    }

    pub async fn region_params_changed(&self, region_params: RegionParams) {
        let _ = self
            .0
            .send(Message::RegionParamsChanged(region_params))
            .await;
    }
}

#[derive(Debug)]
pub struct Beaconer {
    /// keypair to sign reports with
    keypair: Arc<Keypair>,
    /// gateway packet transmit message queue
    transmit: gateway::MessageSender,
    /// Our receive queue.
    messages: MessageReceiver,
    /// Beacon interval
    interval: Duration,
    /// The last beacon that was transitted
    last_beacon: Option<beacon::Beacon>,
    /// Use for channel plan and FR parameters
    region_params: Option<RegionParams>,
    poc_ingest_uri: Uri,
    entropy_service: EntropyService,
}

impl Beaconer {
    pub fn new(
        settings: &Settings,
        transmit: gateway::MessageSender,
        messages: MessageReceiver,
    ) -> Self {
        let interval = {
            let base_interval = settings.poc.interval;
            let max_jitter = (base_interval * BEACON_INTERVAL_JITTER_PERCENTAGE) / 100;
            let jitter = OsRng.gen_range(0..=max_jitter);
            Duration::from_secs(base_interval + jitter)
        };
        let poc_ingest_uri = settings.poc.ingest_uri.clone();
        let entropy_service = EntropyService::new(settings.poc.entropy_uri.clone());
        let keypair = settings.keypair.clone();

        Self {
            keypair,
            transmit,
            messages,
            interval,
            last_beacon: None,
            region_params: None,
            poc_ingest_uri,
            entropy_service,
        }
    }

    pub async fn mk_beacon(&mut self) -> Result<beacon::Beacon> {
        let remote_entropy = self.entropy_service.get_entropy().await?;
        let local_entropy = beacon::Entropy::local()?;
        let region_params = self
            .region_params
            .as_ref()
            .ok_or_else(RegionError::no_region_params)?;

        let beacon = beacon::Beacon::new(remote_entropy, local_entropy, region_params)?;
        Ok(beacon)
    }

    /// Sends a gateway-to-gateway packet.
    ///
    /// See [`gateway::MessageSender::transmit_beacon`]
    pub async fn send_beacon(&mut self, beacon: beacon::Beacon, logger: &Logger) {
        let beacon_id = beacon.beacon_id();
        info!(logger, "transmitting beacon"; "beacon" => &beacon_id);
        let mut report = match poc_lora::LoraBeaconReportReqV1::try_from(beacon.clone()) {
            Ok(report) => report,
            Err(err) => {
                warn!(logger, "failed to construct beacon report {err:?}");
                return;
            }
        };
        let (powe, tmst) = match self.transmit.transmit_beacon(beacon.clone()).await {
            Ok(BeaconResp { powe, tmst }) => (powe, tmst),
            Err(err) => {
                warn!(logger, "failed to transmit beacon {err:?}");
                return;
            }
        };
        self.last_beacon = Some(beacon);
        report.tx_power = powe;
        report.tmst = tmst;
        let _ = PocLoraService::new(self.poc_ingest_uri.clone())
            .submit_beacon(report, self.keypair.clone())
            .inspect_err(|err| info!(logger, "failed to submit poc beacon report: {err:?}"; "beacon" => &beacon_id))
            .inspect_ok(|_| info!(logger, "poc beacon report submitted"; "beacon" => &beacon_id))
            .await;
    }

    async fn handle_message(&mut self, message: Message, logger: &Logger) {
        match message {
            Message::ReceivedBeacon(packet) => self.handle_received_beacon(packet, logger).await,
            Message::RegionParamsChanged(region_params) => {
                self.handle_region_params(region_params, logger)
            }
        }
    }

    async fn mk_witness_report(&self, packet: Packet) -> Result<poc_lora::LoraWitnessReportReqV1> {
        let mut report = packet.to_witness_report()?;
        report.pub_key = self.keypair.public_key().to_vec();
        report.signature = report.sign(self.keypair.clone()).await?;
        Ok(report)
    }

    async fn handle_beacon_tick(&mut self, logger: &Logger) {
        let beacon = match self.mk_beacon().await {
            Ok(beacon) => beacon,
            Err(err) => {
                warn!(logger, "failed to construct beacon: {err:?}");
                return;
            }
        };
        self.send_beacon(beacon, logger).await
    }

    async fn handle_received_beacon(&mut self, packet: Packet, logger: &Logger) {
        info!(logger, "received possible PoC payload: {packet:?}");

        if let Some(last_beacon) = &self.last_beacon {
            if packet.payload == last_beacon.data {
                info!(logger, "ignoring last self beacon witness");
                return;
            }
        }

        let report = match self.mk_witness_report(packet).await {
            Ok(report) => report,
            Err(err) => {
                warn!(logger, "ignoring invalid witness report: {err:?}");
                return;
            }
        };

        let _ = PocLoraService::new(self.poc_ingest_uri.clone())
            .submit_witness(report.clone())
            .inspect_err(|err| info!(logger, "failed to submit poc witness report: {err:?}"; "beacon" => report.data.to_b64()))
            .inspect_ok(|_| info!(logger, "poc witness report submitted"; "beacon" => report.data.to_b64()))
            .await;

        // Disable secondary beacons until TTL is implemented
        if false {
            self.handle_secondary_beacon(report, logger).await
        }
    }

    async fn handle_secondary_beacon(
        &mut self,
        report: poc_lora::LoraWitnessReportReqV1,
        logger: &Logger,
    ) {
        let region_params = match &self.region_params {
            Some(region_params) => region_params,
            None => {
                warn!(logger, "no region params for secondary beacon");
                return;
            }
        };

        // check if hash of witness is below the "difficulty threshold" for a secondary beacon
        // TODO provide a way to get this difficulty threshold from eg. the entropy server
        let buf = report.encode_to_vec();
        let threshold = 1855177858159416090;
        // compare the hash of the witness report as a u64 to the difficulty threshold
        // this is sort of a bitcoin-esque proof of work check insofar as as we're looking
        // for hashes under a certain value. Because of the time constraints involved this
        // should not be a 'mineable' check, but it provides a useful probabalistic way to
        // allow for verifiable secondary beacons without any coordination.
        let factor = xxh64(&buf, 0);
        if factor < threshold {
            let beacon = match beacon::Entropy::from_data(report.data.clone())
                .and_then(|remote_entropy| {
                    beacon::Entropy::from_data(buf)
                        .map(|local_entropy| (remote_entropy, local_entropy))
                })
                .and_then(|(remote_entropy, local_entropy)| {
                    beacon::Beacon::new(remote_entropy, local_entropy, region_params)
                }) {
                Ok(beacon) => beacon,
                Err(err) => {
                    warn!(logger, "secondary beacon construction error: {err:?}");
                    return;
                }
            };

            self.send_beacon(beacon, logger).await
        }
    }

    fn handle_region_params(&mut self, params: RegionParams, logger: &Logger) {
        self.region_params = Some(params);
        info!(logger, "updated region";
              "region" => RegionParams::to_string(&self.region_params));
    }

    /// Enter `Beaconer`'s run loop.
    ///
    /// This routine is will run forever and only returns on error or
    /// shut-down event (.e.g. Control-C, signal).
    pub async fn run(&mut self, shutdown: Listener, logger: &Logger) -> Result {
        let logger = logger.new(slog::o!("module" => "beacon"));
        info!(logger, "starting";  "beacon_interval" => self.interval.as_secs());

        let mut beacon_timer = time::interval(self.interval);

        loop {
            if shutdown.is_triggered() {
                return Ok(());
            }

            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                _ = beacon_timer.tick() => {
                    self.handle_beacon_tick(&logger).await
                },
                message = self.messages.recv() => match message {
                    Some(message) => self.handle_message(message, &logger).await,
                    None => {
                        warn!(logger, "ignoring closed messgae channel");
                    }
                }


            }
        }
    }
}

#[test]
fn test_beacon_roundtrip() {
    use lorawan::PHYPayload;

    let phy_payload_a = PHYPayload::proprietary(b"poc_beacon_data");
    let payload: Vec<u8> = phy_payload_a.clone().try_into().expect("beacon packet");
    let phy_payload_b = PHYPayload::read(lorawan::Direction::Uplink, &mut &payload[..]).unwrap();
    assert_eq!(phy_payload_a, phy_payload_b);
}
