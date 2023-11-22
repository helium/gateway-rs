//! This module provides proof-of-coverage (PoC) beaconing support.
use crate::{
    gateway::{self, BeaconResp},
    message_cache::MessageCache,
    region_watcher,
    service::{entropy::EntropyService, poc::PocIotService, Reconnect},
    settings::Settings,
    sync, Base64, DecodeError, PacketUp, PublicKey, RegionParams, Result,
};
use futures::TryFutureExt;
use helium_proto::services::poc_lora::{self, lora_stream_response_v1};
use http::Uri;
use std::sync::Arc;
use time::{Duration, Instant, OffsetDateTime};
use tracing::{info, warn};

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
    /// gateway packet transmit message queue
    transmit: gateway::MessageSender,
    /// Our receive queue.
    messages: MessageReceiver,
    /// Service to deliver PoC reports to
    service: PocIotService,
    /// Service reconnect trigger
    reconnect: Reconnect,
    /// Region change queue
    region_watch: region_watcher::MessageReceiver,
    /// Beacon interval
    interval: Duration,
    // Time next beacon attempt is to be made
    next_beacon_time: Option<OffsetDateTime>,
    /// Last seen beacons
    last_seen: MessageCache<Vec<u8>>,
    /// Use for channel plan and FR parameters
    region_params: Arc<RegionParams>,
    entropy_uri: Uri,
}

impl Beaconer {
    pub fn new(
        settings: &Settings,
        messages: MessageReceiver,
        region_watch: region_watcher::MessageReceiver,
        transmit: gateway::MessageSender,
    ) -> Self {
        let interval = Duration::seconds(settings.poc.interval as i64);
        let entropy_uri = settings.poc.entropy_uri.clone();
        let service = PocIotService::new(
            "beaconer",
            settings.poc.ingest_uri.clone(),
            settings.keypair.clone(),
        );
        let reconnect = Reconnect::default();
        let region_params = Arc::new(region_watcher::current_value(&region_watch));
        let disabled = settings.poc.disable;

        Self {
            transmit,
            messages,
            region_watch,
            interval,
            last_seen: MessageCache::new(15),
            next_beacon_time: None,
            region_params,
            service,
            entropy_uri,
            disabled,
            reconnect,
        }
    }

    pub async fn run(&mut self, shutdown: &triggered::Listener) -> Result {
        info!(
            beacon_interval = self.interval.whole_seconds(),
            disabled = self.disabled,
            uri = %self.service.uri,
            "starting"
        );

        let mut next_beacon_instant = Instant::now() + self.interval;

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!("shutting down");
                    return Ok(())
                },
                _ = tokio::time::sleep_until(next_beacon_instant.into_inner().into()) => {
                    // Check if beaconing is enabled and we have valid region params
                    if !self.disabled && self.region_params.check_valid().is_ok() {
                        self.handle_beacon_tick().await;
                    }
                    // sleep up to another interval period. A subsequent region
                    // param update will adjust this back to a random offset in
                    // the next valid window
                    next_beacon_instant = Instant::now() + self.interval;
                },
                message = self.messages.recv() => match message {
                    Some(Message::ReceivedBeacon(packet)) => self.handle_received_beacon(packet).await,
                    None => {
                        warn!("ignoring closed message channel");
                    }
                },
                region_change = self.region_watch.changed() => match region_change {
                    Ok(()) => {
                        // Recalculate a potential next beacon time based on the
                        // timestamp in the region parameters.
                        let new_region_params = Arc::new(region_watcher::current_value(&self.region_watch));
                        // new region params can come back with the unknown
                        // region and empty region params. We don't accept
                        // anything but a valid region param before we set a
                        // beacon time
                        if new_region_params.check_valid().is_err() {
                            continue;
                        }
                        // If we can't parse the timestamp ignore the region change altogether
                        let Ok(new_timestamp) = OffsetDateTime::from_unix_timestamp(new_region_params.timestamp as i64) else {
                            continue;
                        };

                        // Calculate next beacon time
                        let new_beacon_time = mk_next_beacon_time(
                            new_timestamp,
                            self.next_beacon_time,
                            self.interval,
                        );

                        // Log next beacon time if changed
                        if Some(new_beacon_time) != self.next_beacon_time {
                            info!(beacon_time = %new_beacon_time, "next beacon time");
                        }
                        self.next_beacon_time = Some(new_beacon_time);
                        next_beacon_instant = Instant::now() + (new_beacon_time - new_timestamp);

                        // Reduce noise, log param change if they actually
                        // changed
                        if  self.region_params != new_region_params {
                            info!(region = RegionParams::to_string(&new_region_params), "region updated");
                        }
                        self.region_params = new_region_params;
                    },
                    Err(_) => warn!("region watch disconnected"),
                },
                service_message = self.service.recv() => match service_message {
                    Ok(lora_stream_response_v1::Response::Offer(message)) => {
                        let session_result = self.handle_session_offer(message).await;
                        if session_result.is_ok() {
                            // (Re)set retry count to max to maximize time to
                            // next disconnect from service
                            self.reconnect.retry_count = self.reconnect.max_retries;
                        } else {
                            // Failed to handle session offer, disconnect
                            self.service.disconnect();
                        }
                        self.reconnect.update_next_time(session_result.is_err());
                    },
                    Err(err) => {
                        warn!(?err, "ingest error");
                        self.reconnect.update_next_time(true);
                    },
                },
                _ = self.reconnect.wait() => {
                    let reconnect_result = self.handle_reconnect().await;
                    self.reconnect.update_next_time(reconnect_result.is_err());
                },

            }
        }
    }

    /// Sends a gateway-to-gateway packet.
    ///
    /// See [`gateway::MessageSender::transmit_beacon`]
    pub async fn send_beacon(&mut self, beacon: beacon::Beacon) -> Result<beacon::Beacon> {
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

        Self::mk_beacon_report(
            beacon.clone(),
            powe,
            tmst,
            self.service.gateway_key().clone(),
        )
        .and_then(|report| self.service.submit_beacon(report))
        .inspect_err(|err| warn!(beacon_id, %err, "submit poc beacon report"))
        .inspect_ok(|_| info!(beacon_id, "poc beacon report submitted"))
        .await?;

        Ok(beacon)
    }

    async fn handle_session_offer(
        &mut self,
        message: poc_lora::LoraStreamSessionOfferV1,
    ) -> Result {
        self.service.session_init(&message.nonce).await
    }

    async fn handle_reconnect(&mut self) -> Result {
        // Do not send waiting reports on ok here since we wait for a session
        // offer. Also do not reset the reconnect retry counter since only a
        // session key indicates a good connection
        self.service
            .reconnect()
            .inspect_err(|err| warn!(%err, "failed to reconnect"))
            .await
    }

    async fn handle_beacon_tick(&mut self) {
        // Need to clone to allow the subsequence borrow of self for send_beacon.
        // The Arc around the region_params makes this a cheap clone
        let region_params = self.region_params.clone();
        let last_beacon = Self::mk_beacon(&region_params, self.entropy_uri.clone())
            .inspect_err(|err| warn!(%err, "construct beacon"))
            .and_then(|beacon| self.send_beacon(beacon))
            .map_ok_or_else(|_| None, Some)
            .await;

        if let Some(data) = last_beacon.beacon_data() {
            self.last_seen.tag_now(data);
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
        if self.last_seen.tag_now(beacon_data.clone()) {
            info!(%beacon_id, "ignoring duplicate or self beacon witness");
            return;
        }

        let _ = Self::mk_witness_report(packet, beacon_data, self.service.gateway_key().clone())
            .and_then(|report| self.service.submit_witness(report))
            .inspect_err(|err| warn!(beacon_id, %err, "submit poc witness report"))
            .inspect_ok(|_| info!(beacon_id, "poc witness report submitted"))
            .await;
    }

    pub async fn mk_beacon(
        region_params: &RegionParams,
        entropy_uri: Uri,
    ) -> Result<beacon::Beacon> {
        region_params.check_valid()?;

        let mut entropy_service = EntropyService::new(entropy_uri);
        let remote_entropy = entropy_service.get_entropy().await?;
        let local_entropy = beacon::Entropy::local()?;

        let beacon = beacon::Beacon::new(remote_entropy, local_entropy, region_params)?;
        Ok(beacon)
    }

    async fn mk_beacon_report(
        beacon: beacon::Beacon,
        conducted_power: i32,
        tmst: u32,
        gateway: PublicKey,
    ) -> Result<poc_lora::LoraBeaconReportReqV1> {
        let mut report = poc_lora::LoraBeaconReportReqV1::try_from(beacon)?;
        report.pub_key = gateway.to_vec();
        report.tx_power = conducted_power;
        report.tmst = tmst;
        Ok(report)
    }

    async fn mk_witness_report(
        packet: PacketUp,
        payload: Vec<u8>,
        gateway: PublicKey,
    ) -> Result<poc_lora::LoraWitnessReportReqV1> {
        let mut report = poc_lora::LoraWitnessReportReqV1::try_from(packet)?;
        report.pub_key = gateway.to_vec();
        report.data = payload;
        Ok(report)
    }
}

fn random_duration(duration: Duration) -> Duration {
    use rand::{rngs::OsRng, Rng};
    Duration::seconds(OsRng.gen_range(0..duration.whole_seconds()))
}

fn mk_next_beacon_time(
    current_time: OffsetDateTime,
    beacon_time: Option<OffsetDateTime>,
    interval: Duration,
) -> OffsetDateTime {
    let current_segment = duration_trunc(current_time, interval);
    let next_segment = current_segment + interval;
    match beacon_time {
        // beacon time is in the future, just use it
        Some(beacon_time) if beacon_time > current_time => beacon_time,
        // beacon time is in the past, either in the current or previous segment
        Some(beacon_time) => {
            let beacon_segment = duration_trunc(beacon_time, interval);
            if beacon_segment == current_segment {
                // current segment: pick a time in the next segment
                next_segment + random_duration(interval)
            } else {
                // previous segment: Pick a time in the remainder of this
                // segment. This really only happens as the current time enters
                // a new segment
                current_time + random_duration(next_segment - current_time)
            }
        }
        // No next beacon time pick a random time in this segment as the beacon
        // time and use this function again
        None => {
            let beacon_time = current_segment + random_duration(interval);
            mk_next_beacon_time(current_time, Some(beacon_time), interval)
        }
    }
}

/// Return a the given time truncated to the nearest duration. Based on
/// duration_trunc in the chrono crate
fn duration_trunc(time: time::OffsetDateTime, duration: Duration) -> time::OffsetDateTime {
    use std::cmp::Ordering;
    let span = duration.whole_seconds().abs();
    let stamp = time.unix_timestamp();
    let delta_down = stamp % span;
    match delta_down.cmp(&0) {
        Ordering::Equal => time,
        Ordering::Greater => time - Duration::seconds(delta_down),
        Ordering::Less => time - Duration::seconds(span - delta_down.abs()),
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

#[cfg(test)]
mod test {
    #[test]
    fn test_beacon_roundtrip() {
        use lorawan::PHYPayload;

        let phy_payload_a = PHYPayload::proprietary(b"poc_beacon_data");
        let payload: Vec<u8> = phy_payload_a.clone().try_into().expect("beacon packet");
        let phy_payload_b =
            PHYPayload::read(lorawan::Direction::Uplink, &mut &payload[..]).unwrap();
        assert_eq!(phy_payload_a, phy_payload_b);
    }

    #[test]
    fn test_beacon_time() {
        use super::{duration_trunc, mk_next_beacon_time};
        use time::{macros::datetime, Duration};

        let interval = Duration::hours(6);
        let current_time = datetime!(2023-09-01 09:20 UTC);

        let current_segment = duration_trunc(current_time, interval);
        let next_segment = current_segment + interval;
        assert!(current_time < next_segment);

        // No beacon time, should pick a time in the remainder of
        // the current segment or in the next segment
        {
            let next_time = mk_next_beacon_time(current_time, None, interval);
            assert!(next_time > current_time);
            assert!(next_time < next_segment + interval);
        }

        // Beacon time in the future
        {
            // In this segment
            let beacon_time = current_time + Duration::minutes(10);
            assert_eq!(current_segment, duration_trunc(beacon_time, interval));
            let next_time = mk_next_beacon_time(current_time, Some(beacon_time), interval);
            assert!(next_time > current_time);
            assert!(next_time < next_segment);
            assert_eq!(current_segment, duration_trunc(next_time, interval));
            // In the next segment
            let beacon_time = next_segment + Duration::minutes(10);
            let next_time = mk_next_beacon_time(current_time, Some(beacon_time), interval);
            assert!(next_time > current_time);
            assert!(next_time > next_segment);
            assert_eq!(next_segment, duration_trunc(next_time, interval));
        }

        // Beacon time in the past
        {
            // This segment, should pick a time in the next segment
            let beacon_time = current_segment + Duration::minutes(10);
            assert!(beacon_time < current_time);
            assert_eq!(current_segment, duration_trunc(beacon_time, interval));
            let next_time = mk_next_beacon_time(current_time, Some(beacon_time), interval);
            assert!(next_time > current_time);
            assert_eq!(next_segment, duration_trunc(next_time, interval));

            // Previous segment, should pick a time in this segment
            let beacon_time = current_segment - Duration::minutes(10);
            let next_time = mk_next_beacon_time(current_time, Some(beacon_time), interval);
            assert!(next_time > current_time);
            assert!(next_time < next_segment);
            assert_eq!(current_segment, duration_trunc(next_time, interval));
        }
    }
}
