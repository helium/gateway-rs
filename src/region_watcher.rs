use crate::{settings::Settings, KeyedUri, Keypair, Region, RegionParams, Result};
use exponential_backoff::Backoff;
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, time};
use tracing::{info, warn};

const REGION_BACKOFF_RETRIES: u32 = 10;
const REGION_BACKOFF_MIN_WAIT: Duration = Duration::from_secs(5);
const REGION_BACKOFF_MAX_WAIT: Duration = Duration::from_secs(3600); // 60 minutes

pub type MessageSender = watch::Sender<RegionParams>;
pub type MessageReceiver = watch::Receiver<RegionParams>;

pub fn current_value<T>(receiver: &watch::Receiver<T>) -> T
where
    T: Clone,
{
    receiver.borrow().clone()
}

pub struct RegionWatcher {
    keypair: Arc<Keypair>,
    config_uri: KeyedUri,
    default_region: Region,
    request_retry: u32,
    watch: MessageSender,
}

impl RegionWatcher {
    pub fn new(settings: &Settings) -> Self {
        let default_params = RegionParams::from(settings.region);
        let (watch, _) = watch::channel(default_params);
        Self {
            keypair: settings.keypair.clone(),
            config_uri: settings.config.clone(),
            // Start retry at 1 to get some jitter in the first request time
            request_retry: 1,
            default_region: settings.region,
            watch,
        }
    }

    pub fn watcher(&mut self) -> watch::Receiver<RegionParams> {
        self.watch.subscribe()
    }

    pub async fn run(&mut self, shutdown: &triggered::Listener) -> Result {
        info!(
            default_region = %self.default_region,
            "starting",
        );

        let backoff = Backoff::new(
            REGION_BACKOFF_RETRIES,
            REGION_BACKOFF_MIN_WAIT,
            REGION_BACKOFF_MAX_WAIT,
        );

        loop {
            let sleep = backoff
                .next(self.request_retry)
                .unwrap_or(REGION_BACKOFF_MAX_WAIT);

            tokio::select! {
                _ = shutdown.clone() => {
                    info!("shutting down");
                    return Ok(())
                },
                _ = time::sleep(sleep) => match self.check_region(shutdown).await {
                    // A successful fetch will set request_retry to RETRIES + 1
                    // which means a first error can reset it back to 1 to start
                    // backing of up to RETRIES
                    Err(_) => self.request_retry = if self.request_retry > REGION_BACKOFF_RETRIES {
                        1
                    } else {
                        (self.request_retry + 1).min(REGION_BACKOFF_RETRIES)
                    },
                    Ok(None) => (),
                    Ok(Some(remote_params)) => {
                        self.request_retry = REGION_BACKOFF_RETRIES + 1;
                        if remote_params != *self.watch.borrow() {
                            _ = self.watch.send_replace(remote_params);
                        };
                    },
                }
            }
        }
    }

    pub async fn check_region(
        &mut self,
        shutdown: &triggered::Listener,
    ) -> Result<Option<RegionParams>> {
        let mut service = crate::service::config::ConfigService::new(&self.config_uri);
        let current_region = self.watch.borrow().region;
        let service_uri = service.uri.clone();

        tokio::select! {
            _ = shutdown.clone() => Ok(None),
            response = service.region_params(current_region, self.keypair.clone()) => match response.map(Some) {
                Err(err) => {
                    warn!(
                        pubkey = %service_uri.pubkey,
                        uri = %service_uri.uri,
                        default_region = %current_region,
                        %err,
                        "failed to get region_params"
                    );
                    Err(err)
                }
                Ok(other) => {
                    let region = other.as_ref().map(|params| params.region).unwrap_or_default();
                    info!(
                        pubkey = %service_uri.pubkey,
                        uri = %service_uri.uri,
                        default_region = %current_region,
                        %region,
                        "fetched config region_params",
                    );
                    Ok(other)
                }
        }
        }
    }
}
