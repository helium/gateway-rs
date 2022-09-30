//! This module provides proof-of-coverage (PoC) beaconing support.
//!
//! TODO: where to get beacon interval from?
//!
//! TODO: fuzz beacon interval to prevent thundering herd.

use crate::{
    gateway,
    service::{entropy::EntropyService, poc::PocLoraService},
    settings::Settings,
    sync, Base64, Error, Keypair, Packet, RegionParams, Result,
};
use futures::TryFutureExt;
use helium_proto::services::poc_lora;
use slog::{self, info, warn, Logger};
use std::{sync::Arc, time::Duration};
use tokio::time;
use triggered::Listener;

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
    /// Use for channel plan and FR parameters
    region_params: Option<RegionParams>,
    poc_service: PocLoraService,
    entropy_service: EntropyService,
}

impl Beaconer {
    pub fn new(
        settings: &Settings,
        transmit: gateway::MessageSender,
        messages: MessageReceiver,
    ) -> Self {
        let interval = Duration::from_secs(settings.poc.beacon_interval);
        let poc_service = PocLoraService::new(settings.poc.ingest_uri.clone());
        let entropy_service = EntropyService::new(settings.poc.entropy_uri.clone());
        let keypair = settings.keypair.clone();

        Self {
            keypair,
            transmit,
            messages,
            interval,
            region_params: None,
            poc_service,
            entropy_service,
        }
    }

    pub async fn mk_beacon(&mut self) -> Result<beacon::Beacon> {
        let remote_entropy = self.entropy_service.get_entropy().await?;
        let local_entropy = beacon::Entropy::local()?;

        let region_params = if let Some(region_params) = &self.region_params {
            region_params
        } else {
            return Err(Error::custom("no region set"));
        };
        let beacon = beacon::Beacon::new(remote_entropy, local_entropy, region_params.as_ref())?;
        Ok(beacon)
    }

    /// Sends a gateway-to-gateway packet.
    ///
    /// See [`gateway::MessageSender::transmit_beacon`]
    pub async fn send_beacon(&mut self, logger: &Logger) {
        let beacon = match self.mk_beacon().await {
            Ok(beacon) => beacon,
            Err(err) => {
                warn!(logger, "failed to construct beacon: {err:?}");
                return;
            }
        };
        let beacon_id = beacon.beacon_id();
        info!(logger, "transmitting beacon"; "beacon" => &beacon_id);
        let report = match poc_lora::LoraBeaconReportReqV1::try_from(beacon.clone()) {
            Ok(report) => report,
            Err(err) => {
                warn!(logger, "failed to construct beack report {err:?}");
                return;
            }
        };
        self.transmit.transmit_beacon(beacon).await;

        let _ =  self
            .poc_service
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

    async fn handle_received_beacon(&mut self, packet: Packet, logger: &Logger) {
        info!(logger, "received possible PoC payload: {packet:?}");
        let report = match packet.to_witness_report() {
            Ok(report) => report,
            Err(err) => {
                warn!(logger, "ignoring invalid witness report: {err:?}");
                return;
            }
        };
        let _ = self
            .poc_service
            .submit_witness(report.clone(), self.keypair.clone())
            .inspect_err(|err| info!(logger, "failed to submit poc witness report: {err:?}"; "beacon" => report.data.to_b64()))
            .inspect_ok(|_| info!(logger, "poc witness report submitted"; "beacon" => report.data.to_b64()))
            .await;
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
        info!(logger, "starting");

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
                _ = beacon_timer.tick() => self.send_beacon(&logger).await,
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

    let phy_payload_a = PHYPayload::propietary(b"poc_beacon_data");
    let payload: Vec<u8> = phy_payload_a.clone().try_into().expect("beacon packet");
    let phy_payload_b = PHYPayload::read(lorawan::Direction::Uplink, &mut &payload[..]).unwrap();
    assert_eq!(phy_payload_a, phy_payload_b);
}
