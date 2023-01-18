use crate::{service, settings::Settings, KeyedUri, Keypair, Region, RegionParams, Result};
use exponential_backoff::Backoff;
use slog::{info, o, warn, Logger};
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, time};

const REGION_BACKOFF_RETRIES: u32 = 10;
const REGION_BACKOFF_MIN_WAIT: Duration = Duration::from_secs(60);
const REGION_BACKOFF_MAX_WAIT: Duration = Duration::from_secs(3600); // 60 minutes

pub type MessageSender = watch::Sender<Option<RegionParams>>;
pub type MessageReceiver = watch::Receiver<Option<RegionParams>>;

pub struct RegionWatcher {
    keypair: Arc<Keypair>,
    default_region: Region,
    config_uri: KeyedUri,
    request_retry: u32,
    watch: MessageSender,
}

impl RegionWatcher {
    pub fn new(settings: &Settings) -> Self {
        let (watch, _) = watch::channel(None);
        Self {
            keypair: settings.keypair.clone(),
            default_region: settings.region,
            config_uri: settings.config.clone(),
            // Start retry at 1 to get some jitter in the first request time
            request_retry: 1,
            watch,
        }
    }

    pub fn watcher(&mut self) -> watch::Receiver<Option<RegionParams>> {
        self.watch.subscribe()
    }

    pub async fn run(&mut self, shutdown: &triggered::Listener, logger: &Logger) -> Result {
        let logger = logger.new(o!("module" => "region_watcher"));
        info!(logger, "starting"; 
            "default_region" => self.default_region);

        let backoff = Backoff::new(
            REGION_BACKOFF_RETRIES,
            REGION_BACKOFF_MIN_WAIT,
            REGION_BACKOFF_MAX_WAIT,
        );

        loop {
            // Check if shutdown trigger already happened
            if shutdown.is_triggered() {
                return Ok(());
            }

            let sleep = backoff
                .next(self.request_retry)
                .unwrap_or(REGION_BACKOFF_MAX_WAIT);

            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                _ = time::sleep(sleep) => match self.check_region(shutdown).await {
                    Err(err) => {
                        warn!(logger, "failed to check region {err:?}");
                        self.request_retry += 1;
                    },
                    Ok(remote_params) => {
                        // Reset to a small random offset from the interval
                        self.request_retry = 1;
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
        let mut service = service::config::ConfigService::new(&self.config_uri)?;
        let region = self
            .watch
            .borrow()
            .as_ref()
            .map_or_else(|| self.default_region, |params| params.region);

        tokio::select! {
            _ = shutdown.clone() => Ok(None),
            response = service.region_params(region, self.keypair.clone()) => response.map(Some)
        }
    }
}
