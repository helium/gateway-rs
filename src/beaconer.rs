//! This module provides proof-of-coverage (PoC) beaconing support.

use crate::{
    gateway::{self, BeaconResp},
    region_watcher,
    service::{entropy::EntropyService, poc::PocIotService},
    settings::Settings,
    sign, sync, Base64, Keypair, PacketUp, RegionParams, Result,
};
use futures::TryFutureExt;
use helium_proto::{services::poc_lora, Message as ProtoMessage};
use http::Uri;
use rand::{rngs::OsRng, Rng};
use std::sync::Arc;
use tokio::time::{self, Duration, Instant};
use tracing::{info, warn};
use xxhash_rust::xxh64::xxh64;

/// To prevent a thundering herd of hotspots all beaconing at the same time, we
/// add a randomized jitter value of up to `BEACON_INTERVAL_JITTER_PERCENTAGE`
/// to the configured beacon interval. This jitter factor is one time only, and
/// will only change when this process or task restarts.
const BEACON_INTERVAL_JITTER_PERCENTAGE: u64 = 10;

/// Message types that can be sent to `Beaconer`'s inbox.
#[derive(Debug)]
pub enum Message {
    ReceivedBeacon(PacketUp),
}

pub type MessageSender = sync::MessageSender<Message>;
pub type MessageReceiver = sync::MessageReceiver<Message>;

pub fn message_channel() -> (MessageSender, MessageReceiver) {
    sync::message_channel(10)
}

impl MessageSender {
    pub async fn received_beacon(&self, packet: PacketUp) {
        self.send(Message::ReceivedBeacon(packet)).await
    }
}

pub struct Beaconer {
    /// Beacon/Witness handling disabled
    disabled: bool,
    /// keypair to sign reports with
    keypair: Arc<Keypair>,
    /// gateway packet transmit message queue
    transmit: gateway::MessageSender,
    /// Our receive queue.
    messages: MessageReceiver,
    /// Region change queue
    region_watch: region_watcher::MessageReceiver,
    /// Beacon interval
    interval: Duration,
    // Time next beacon attempt is o be made
    next_beacon_time: Instant,
    /// The last beacon that was transitted
    last_beacon: Option<beacon::Beacon>,
    /// Use for channel plan and FR parameters
    region_params: RegionParams,
    poc_ingest_uri: Uri,
    entropy_uri: Uri,
}

impl Beaconer {
    pub fn new(
        settings: &Settings,
        messages: MessageReceiver,
        region_watch: region_watcher::MessageReceiver,
        transmit: gateway::MessageSender,
    ) -> Self {
        let interval = Duration::from_secs(settings.poc.interval);
        let poc_ingest_uri = settings.poc.ingest_uri.clone();
        let entropy_uri = settings.poc.entropy_uri.clone();
        let keypair = settings.keypair.clone();
        let region_params = region_watcher::current_value(&region_watch);
        let disabled = settings.poc.disable;

        Self {
            keypair,
            transmit,
            messages,
            region_watch,
            interval,
            last_beacon: None,
            // Set a beacon at least an interval out... arrival of region_params
            // will recalculate this time and no arrival of region_params will
            // cause the beacon to not occur
            next_beacon_time: Instant::now() + interval,
            region_params,
            poc_ingest_uri,
            entropy_uri,
            disabled,
        }
    }

    pub async fn run(&mut self, shutdown: &triggered::Listener) -> Result {
        info!(
            beacon_interval = self.interval.as_secs(),
            disabled = self.disabled,
            "starting"
        );

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!("shutting down");
                    return Ok(())
                },
                _ = time::sleep_until(self.next_beacon_time) => {
                    self.handle_beacon_tick().await
                },
                message = self.messages.recv() => match message {
                    Some(Message::ReceivedBeacon(packet)) => self.handle_received_beacon(packet).await,
                    None => {
                        warn!("ignoring closed message channel");
                    }
                },
                region_change = self.region_watch.changed() => match region_change {
                    Ok(()) => {
                        // Recalculate beacon time based on if this
                        // was the first time region params have
                        // arrived.  Ensure that the next beacon
                        // time is not the full interval if this is
                        // not the first region change
                        //
                        // Do the first time check below before
                        // region params are assigned
                        if self.region_params.params.is_empty() {
                            self.next_beacon_time =
                                Self::mk_next_short_beacon_time(self.interval);
                        }
                        self.region_params = region_watcher::current_value(&self.region_watch);
                        info!(region = RegionParams::to_string(&self.region_params), "region updated");
                    },
                    Err(_) => warn!("region watch disconnected"),
                }


            }
        }
    }

    pub async fn mk_beacon(&mut self) -> Result<beacon::Beacon> {
        self.region_params.check_valid()?;

        let mut entropy_service = EntropyService::new(self.entropy_uri.clone());
        let remote_entropy = entropy_service.get_entropy().await?;
        let local_entropy = beacon::Entropy::local()?;

        let beacon = beacon::Beacon::new(remote_entropy, local_entropy, &self.region_params)?;
        Ok(beacon)
    }

    /// Sends a gateway-to-gateway packet.
    ///
    /// See [`gateway::MessageSender::transmit_beacon`]
    pub async fn send_beacon(&mut self, beacon: beacon::Beacon) {
        let beacon_id = beacon.beacon_id();
        info!(beacon_id, "transmitting beacon");

        let (powe, tmst) = match self.transmit.transmit_beacon(beacon.clone()).await {
            Ok(BeaconResp { powe, tmst }) => (powe, tmst),
            Err(err) => {
                warn!(%err, "transmit beacon");
                return;
            }
        };

        self.last_beacon = Some(beacon.clone());

        let report = match self.mk_beacon_report(beacon, powe, tmst).await {
            Ok(report) => report,
            Err(err) => {
                warn!(beacon_id, %err, "poc beacon report");
                return;
            }
        };
        let _ = PocIotService::new(self.poc_ingest_uri.clone())
            .submit_beacon(report)
            .inspect_err(|err| info!(beacon_id, %err, "submit poc beacon report",))
            .inspect_ok(|_| info!(beacon_id, "poc beacon report submitted",))
            .await;
    }

    async fn mk_beacon_report(
        &self,
        beacon: beacon::Beacon,
        conducted_power: i32,
        tmst: u32,
    ) -> Result<poc_lora::LoraBeaconReportReqV1> {
        let mut report = poc_lora::LoraBeaconReportReqV1::try_from(beacon)?;
        report.tx_power = conducted_power;
        report.tmst = tmst;
        report.pub_key = self.keypair.public_key().to_vec();
        report.signature = sign(self.keypair.clone(), report.encode_to_vec()).await?;
        Ok(report)
    }

    async fn mk_witness_report(
        &self,
        packet: PacketUp,
    ) -> Result<poc_lora::LoraWitnessReportReqV1> {
        let mut report = poc_lora::LoraWitnessReportReqV1::try_from(packet)?;
        report.pub_key = self.keypair.public_key().to_vec();
        report.signature = sign(self.keypair.clone(), report.encode_to_vec()).await?;
        Ok(report)
    }

    async fn handle_beacon_tick(&mut self) {
        if self.disabled {
            return;
        }
        match self.mk_beacon().await {
            Ok(beacon) => {
                self.send_beacon(beacon).await;
                // On success just use the normal behavior for selecting a next
                // beacon time. Can't be the first time since we have region
                // parameters to construct a beacon
                self.next_beacon_time = Self::mk_next_beacon_time(self.interval);
            }
            Err(err) => {
                warn!(%err, "construct beacon");
                // On failure to construct a beacon at all, select a shortened
                // "first time" next beacon time
                self.next_beacon_time = Self::mk_next_short_beacon_time(self.interval);
            }
        };
    }

    async fn handle_received_beacon(&mut self, packet: PacketUp) {
        if self.disabled {
            return;
        }
        if let Some(last_beacon) = &self.last_beacon {
            if packet.payload() == last_beacon.data {
                info!("ignoring last self beacon witness");
                return;
            }
        }

        let report = match self.mk_witness_report(packet).await {
            Ok(report) => report,
            Err(err) => {
                warn!(%err, "ignoring invalid witness report");
                return;
            }
        };

        let _ = PocIotService::new(self.poc_ingest_uri.clone())
            .submit_witness(report.clone())
            .inspect_err(|err| {
                info!(
                    beacon = report.data.to_b64(),
                    %err,
                    "submit poc witness report"
                )
            })
            .inspect_ok(|_| {
                info!(
                    beacon = report.data.to_b64(),
                    "poc witness report submitted"
                )
            })
            .await;

        // Disable secondary beacons until TTL is implemented
        if false {
            self.handle_secondary_beacon(report).await
        }
    }

    async fn handle_secondary_beacon(&mut self, report: poc_lora::LoraWitnessReportReqV1) {
        if self.region_params.check_valid().is_err() {
            warn!("no region params for secondary beacon");
            return;
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
                    beacon::Beacon::new(remote_entropy, local_entropy, &self.region_params)
                }) {
                Ok(beacon) => beacon,
                Err(err) => {
                    warn!(%err, "secondary beacon construction");
                    return;
                }
            };

            self.send_beacon(beacon).await
        }
    }

    /// Construct a next beacon time based on a fraction of the given interval.
    fn mk_next_short_beacon_time(interval: Duration) -> Instant {
        let now = Instant::now();
        let max_jitter = (interval.as_secs() * BEACON_INTERVAL_JITTER_PERCENTAGE) / 100;
        let jitter = OsRng.gen_range(0..=max_jitter);
        now + Duration::from_secs(jitter)
    }

    /// Construct a next beacon time based on the current time and given interval.
    fn mk_next_beacon_time(interval: Duration) -> Instant {
        let now = Instant::now();
        now + interval
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
