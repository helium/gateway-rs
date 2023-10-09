//! This module provides proof-of-coverage (PoC) beaconing support.

use crate::{
    error::DecodeError,
    gateway::{self, BeaconResp},
    keypair::mk_session_keypair,
    message_cache::MessageCache,
    region_watcher,
    service::{entropy::EntropyService, poc::PocIotService, Reconnect},
    settings::Settings,
    sign, sync, Base64, Error, Keypair, PacketUp, PublicKey, RegionParams, Result,
};
use futures::TryFutureExt;
use helium_proto::{
    services::poc_lora::{self, lora_stream_request_v1, lora_stream_response_v1},
    Message as ProtoMessage,
};
use http::Uri;
use std::sync::Arc;
use time::{Duration, Instant};
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
    /// keypair to sign session init with
    keypair: Arc<Keypair>,
    /// session keypair to use for reports
    session_key: Option<Arc<Keypair>>,
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
    next_beacon_time: Instant,
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
        let service = PocIotService::new(settings.poc.ingest_uri.clone(), settings.keypair.clone());
        let keypair = settings.keypair.clone();
        let reconnect = Reconnect::default();
        let region_params = Arc::new(region_watcher::current_value(&region_watch));
        let disabled = settings.poc.disable;

        Self {
            keypair,
            session_key: None,
            transmit,
            messages,
            region_watch,
            interval,
            last_seen: MessageCache::new(15),
            // Set a beacon at least an interval out... arrival of region_params
            // will recalculate this time and no arrival of region_params will
            // cause the beacon to not occur
            next_beacon_time: Instant::now() + interval,
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
            "starting"
        );

        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!("shutting down");
                    return Ok(())
                },
                _ = tokio::time::sleep_until(self.next_beacon_time.into_inner().into()) => {
                    self.handle_beacon_tick().await;
                    self.next_beacon_time += self.interval;
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
                        let new_region_params = region_watcher::current_value(&self.region_watch);

                        if self.region_params.params.is_empty() {
                            // Calculate a random but deterministic time offset
                            // for this hotspot's beacons
                            let offset = mk_beacon_offset(self.keypair.public_key(), self.interval);
                            // Get a delay for the first beacon based on the
                            // deterministic offset and the timestamp in the
                            // first region params. If there's an error
                            // converting the region param timestamp the
                            // calculated offset
                            let delay = mk_first_beacon_delay(new_region_params.timestamp, self.interval, offset).unwrap_or(offset);
                            info!(delay = delay.whole_seconds(), "first beacon");
                            self.next_beacon_time = Instant::now() + delay;
                        }
                        self.region_params = Arc::new(region_watcher::current_value(&self.region_watch));
                        info!(region = RegionParams::to_string(&self.region_params), "region updated");
                    },
                    Err(_) => warn!("region watch disconnected"),
                },
                service_message = self.service.recv() => match service_message {
                    Ok(Some(lora_stream_response_v1::Response::Offer(message))) => {
                        let session_result = self.handle_session_offer(message).await;
                        if session_result.is_ok() {
                            // (Re)set retry count to max to maximize time to
                            // next disconnect from service
                            self.reconnect.retry_count = self.reconnect.max_retries;
                        } else {
                            // Failed to handle session offer, disconnect
                            self.disconnect();
                        }
                        self.reconnect.update_next_time(session_result.is_err());
                    },
                    Ok(None) => {
                        warn!("ingest disconnected");
                        self.reconnect.update_next_time(true);
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

        // Check if a session key is available to sign the report
        let Some(session_key) = self.session_key.clone() else {
            warn!(%beacon_id, "no session key for beacon report");
            return Err(Error::no_service());
        };

        Self::mk_beacon_report(beacon.clone(), powe, tmst, session_key)
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
        let session_key = mk_session_key_init(self.keypair.clone(), &message)
            .and_then(|(session_key, session_init)| {
                self.service.send(session_init).map_ok(|_| session_key)
            })
            .inspect_err(|err| warn!(%err, "failed to initialize session"))
            .await?;
        self.session_key = Some(session_key.clone());
        info!(session_key = %session_key.public_key(),"initialized session");
        Ok(())
    }

    async fn handle_reconnect(&mut self) -> Result {
        // Do not send waiting reports on ok here since we wait for a sesson
        // offer. Also do not reset the reconnect retry counter since only a
        // session key indicates a good connection
        self.service
            .reconnect()
            .inspect_err(|err| warn!(%err, "failed to reconnect"))
            .await
    }

    async fn handle_beacon_tick(&mut self) {
        if self.disabled {
            return;
        }

        let last_beacon = Self::mk_beacon(self.region_params.clone(), self.entropy_uri.clone())
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

        // Check if a session key is available to sign the report
        let Some(session_key) = self.session_key.clone() else {
            warn!(%beacon_id, "no session key for witness report");
            return;
        };

        let _ = Self::mk_witness_report(packet, beacon_data, session_key)
            .and_then(|report| self.service.submit_witness(report))
            .inspect_err(|err| warn!(beacon_id, %err, "submit poc witness report"))
            .inspect_ok(|_| info!(beacon_id, "poc witness report submitted"))
            .await;
    }

    fn disconnect(&mut self) {
        self.service.disconnect();
        self.session_key = None;
    }

    pub async fn mk_beacon(
        region_params: Arc<RegionParams>,
        entropy_uri: Uri,
    ) -> Result<beacon::Beacon> {
        region_params.check_valid()?;

        let mut entropy_service = EntropyService::new(entropy_uri);
        let remote_entropy = entropy_service.get_entropy().await?;
        let local_entropy = beacon::Entropy::local()?;

        let beacon = beacon::Beacon::new(remote_entropy, local_entropy, &region_params)?;
        Ok(beacon)
    }

    async fn mk_beacon_report(
        beacon: beacon::Beacon,
        conducted_power: i32,
        tmst: u32,
        keypair: Arc<Keypair>,
    ) -> Result<poc_lora::LoraBeaconReportReqV1> {
        let mut report = poc_lora::LoraBeaconReportReqV1::try_from(beacon)?;
        report.tx_power = conducted_power;
        report.tmst = tmst;
        report.pub_key = keypair.public_key().to_vec();
        report.signature = sign(keypair.clone(), report.encode_to_vec()).await?;
        Ok(report)
    }

    async fn mk_witness_report(
        packet: PacketUp,
        payload: Vec<u8>,
        keypair: Arc<Keypair>,
    ) -> Result<poc_lora::LoraWitnessReportReqV1> {
        let mut report = poc_lora::LoraWitnessReportReqV1::try_from(packet)?;
        report.data = payload;
        report.pub_key = keypair.public_key().to_vec();
        report.signature = sign(keypair.clone(), report.encode_to_vec()).await?;
        Ok(report)
    }
}

pub async fn mk_session_key_init(
    keypair: Arc<Keypair>,
    offer: &poc_lora::LoraStreamSessionOfferV1,
) -> Result<(Arc<Keypair>, lora_stream_request_v1::Request)> {
    let session_keypair = Arc::new(mk_session_keypair());
    let session_key = session_keypair.public_key();

    let mut session_init = poc_lora::LoraStreamSessionInitV1 {
        pub_key: keypair.public_key().into(),
        session_key: session_key.into(),
        nonce: offer.nonce.clone(),
        signature: vec![],
    };
    session_init.signature = sign(keypair, session_init.encode_to_vec()).await?;
    let envelope = lora_stream_request_v1::Request::SessionInit(session_init);
    Ok((session_keypair, envelope))
}

/// Construct a random but deterministic offset for beaconing. This is based on
/// the public key as of this hotspot as the seed to a random number generator.
fn mk_beacon_offset(key: &PublicKey, interval: Duration) -> Duration {
    use rand::{Rng, SeedableRng};
    use sha2::Digest;

    let hash = sha2::Sha256::digest(key.to_vec());
    let mut rng = rand::rngs::StdRng::from_seed(*hash.as_ref());
    Duration::seconds(rng.gen_range(0..interval.whole_seconds()))
}

/// Construct the first beacon time. This positions the given offset in the next
/// interval based wall clock segment. It returns the time to sleep until that
/// determinstic offset in the current or next segment.
fn mk_first_beacon_delay(
    current_time: u64,
    interval: Duration,
    offset: Duration,
) -> Option<Duration> {
    time::OffsetDateTime::from_unix_timestamp(current_time as i64)
        .map(|now| {
            let current_segment = duration_trunc(now, interval);
            let mut first_time = current_segment + offset;
            if first_time < now {
                first_time += interval;
            }
            first_time - now
        })
        .ok()
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
    fn test_beacon_offset() {
        use super::mk_beacon_offset;
        use std::str::FromStr;

        const PUBKEY_1: &str = "13WvV82S7QN3VMzMSieiGxvuaPKknMtf213E5JwPnboDkUfesKw";
        const PUBKEY_2: &str = "14HZVR4bdF9QMowYxWrumcFBNfWnhDdD5XXA5za1fWwUhHxxFS1";
        let pubkey_1 = helium_crypto::PublicKey::from_str(PUBKEY_1).expect("public key");
        let offset_1 = mk_beacon_offset(&pubkey_1, time::Duration::hours(6));
        // Same key and interval should always end up at the same offset
        assert_eq!(
            offset_1,
            mk_beacon_offset(&pubkey_1, time::Duration::hours(6))
        );
        let pubkey_2 = helium_crypto::PublicKey::from_str(PUBKEY_2).expect("public key 2");
        let offset_2 = mk_beacon_offset(&pubkey_2, time::Duration::hours(6));
        assert_eq!(
            offset_2,
            mk_beacon_offset(&pubkey_2, time::Duration::hours(6))
        );
        // And two offsets based on different keys should not land at the same
        // offset
        assert_ne!(offset_1, offset_2);
    }

    #[test]
    fn test_beacon_first_time() {
        use super::mk_first_beacon_delay;
        use time::{macros::datetime, Duration};

        let interval = Duration::hours(6);
        let early_offset = Duration::minutes(10);
        let late_offset = early_offset + Duration::hours(5);

        let current_time = datetime!(2023-09-01 09:20 UTC);
        let early_sleep =
            mk_first_beacon_delay(current_time.unix_timestamp() as u64, interval, early_offset)
                .unwrap_or(early_offset);
        let late_sleep =
            mk_first_beacon_delay(current_time.unix_timestamp() as u64, interval, late_offset)
                .unwrap_or(late_offset);

        assert_eq!(
            datetime!(2023-09-01 12:10:00 UTC),
            current_time + early_sleep
        );
        assert_eq!(
            datetime!(2023-09-01 11:10:00 UTC),
            current_time + late_sleep
        );
    }
}
