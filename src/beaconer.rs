//! This module provides proof-of-coverage (PoC) beaconing support.

use crate::{
    error::DecodeError,
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
use std::{num::NonZeroUsize, sync::Arc};
use tokio::time::{self, Duration, Instant};
use tracing::{info, warn};

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

#[derive(Debug)]
struct SeenCache(lru::LruCache<Vec<u8>, bool>);

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
    /// Last seen beacons
    last_seen: SeenCache,
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
            last_seen: SeenCache::new(10),
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

    pub async fn mk_beacon(&self) -> Result<beacon::Beacon> {
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
    pub async fn send_beacon(&self, beacon: beacon::Beacon) -> Result<beacon::Beacon> {
        let beacon_id = beacon
            .beacon_data()
            .map(|data| data.to_b64())
            .ok_or_else(DecodeError::not_beacon)?;

        info!(beacon_id, "transmitting beacon");

        let (powe, tmst) = self
            .transmit
            .transmit_beacon(beacon.clone())
            .inspect_err(|err| warn!(%err, "transmit beacon"))
            .map_ok(|BeaconResp { powe, tmst }| (powe, tmst))
            .await?;

        // Construct concurrent futures for connecting to the poc ingester and
        // signing the report
        let report_fut = self.mk_beacon_report(beacon.clone(), powe, tmst);
        let service_fut = PocIotService::connect(self.poc_ingest_uri.clone());

        match tokio::try_join!(report_fut, service_fut) {
            Ok((report, mut poc_service)) => {
                poc_service
                    .submit_beacon(report)
                    .inspect_err(|err| warn!(beacon_id, %err, "submit poc beacon report"))
                    .inspect_ok(|_| info!(beacon_id, "poc beacon report submitted"))
                    .await?
            }
            Err(err) => {
                warn!(beacon_id, %err, "poc beacon report");
            }
        }

        Ok(beacon)
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
        payload: Vec<u8>,
    ) -> Result<poc_lora::LoraWitnessReportReqV1> {
        let mut report = poc_lora::LoraWitnessReportReqV1::try_from(packet)?;
        report.data = payload;
        report.pub_key = self.keypair.public_key().to_vec();
        report.signature = sign(self.keypair.clone(), report.encode_to_vec()).await?;
        Ok(report)
    }

    async fn handle_beacon_tick(&mut self) {
        if self.disabled {
            return;
        }
        let interval = self.interval;
        let (last_beacon, next_beacon_time) = self
            .mk_beacon()
            .inspect_err(|err| warn!(%err, "construct beacon"))
            .and_then(|beacon| self.send_beacon(beacon))
            // On success to construct and transmit a beacon and its report
            // select a normal full next beacon time
            .map_ok(|beacon| (Some(beacon), Self::mk_next_beacon_time(interval)))
            // On failure to construct, transmit or send a beacon or its
            // report, select a shortened next beacon time
            .unwrap_or_else(|_| (None, Self::mk_next_short_beacon_time(interval)))
            .await;

        self.next_beacon_time = next_beacon_time;

        if let Some(data) = last_beacon.beacon_data() {
            self.last_seen.tag(data);
        }
    }

    async fn handle_received_beacon(&mut self, packet: PacketUp) {
        // Check if poc reporting is disabled
        if self.disabled {
            return;
        }

        // Check that there is beacon data present
        let Some(beacon_data) = packet.beacon_data() else {
            warn!("ignoring invalid received beacon");
            return;
        };

        let beacon_id = beacon_data.to_b64();

        // Check if we've seen this beacon before
        if self.last_seen.tag(beacon_data.clone()) {
            info!(%beacon_id, "ignoring duplicate or self beacon witness");
            return;
        }

        // Construct concurrent futures for connecting to the poc ingester and
        // signing the report
        let report_fut = self.mk_witness_report(packet, beacon_data);
        let service_fut = PocIotService::connect(self.poc_ingest_uri.clone());

        match tokio::try_join!(report_fut, service_fut) {
            Ok((report, mut poc_service)) => {
                let _ = poc_service
                    .submit_witness(report)
                    .inspect_err(|err| warn!(beacon_id, %err, "submit poc witness report"))
                    .inspect_ok(|_| info!(beacon_id, "poc witness report submitted"))
                    .await;
            }
            Err(err) => {
                warn!(%err, "poc witness report");
            }
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

trait BeaconData {
    fn beacon_data(&self) -> Option<Vec<u8>>;
}

impl BeaconData for PacketUp {
    fn beacon_data(&self) -> Option<Vec<u8>> {
        match PacketUp::parse_frame(lorawan::Direction::Uplink, self.payload()) {
            Ok(lorawan::PHYPayloadFrame::Proprietary(payload)) => Some(payload.into()),
            _ => None,
        }
    }
}

impl BeaconData for beacon::Beacon {
    fn beacon_data(&self) -> Option<Vec<u8>> {
        Some(self.data.clone())
    }
}

impl BeaconData for Option<beacon::Beacon> {
    fn beacon_data(&self) -> Option<Vec<u8>> {
        self.as_ref().and_then(|beacon| beacon.beacon_data())
    }
}

impl SeenCache {
    fn new(capacity: usize) -> Self {
        Self(lru::LruCache::new(
            NonZeroUsize::new(capacity).expect("cache capacity > 0"),
        ))
    }

    /// Checks the cache for a given key. If the key is not present, the key is
    /// inserted, if it does exist it's moved to the head of the lru cache to
    /// keep it fresh
    fn tag(&mut self, key: Vec<u8>) -> bool {
        *self.0.get_or_insert(key, || true)
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
