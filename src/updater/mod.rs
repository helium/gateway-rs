pub mod releases;

use crate::{settings, Result, Settings};
use futures::TryStreamExt;
use http::Uri;
use releases::Channel;
use slog::{error, info, o, warn, Logger};
use std::{
    env, io,
    path::{Path, PathBuf},
};
use tokio::{process, time};

#[derive(Debug)]
pub struct Updater {
    enabled: bool,
    uri: Uri,
    channel: Channel,
    platform: String,
    interval: time::Duration,
    install_command: String,
}

impl Updater {
    pub fn new(settings: &Settings) -> Result<Self> {
        Ok(Self {
            enabled: settings.update.enabled,
            channel: settings.update.channel.clone(),
            platform: settings.update.platform.clone(),
            interval: time::Duration::from_secs(settings.update.interval as u64 * 60),
            uri: settings.update.uri.clone(),
            install_command: settings.update.command.clone(),
        })
    }

    pub async fn run(&self, shutdown: triggered::Listener, logger: &Logger) -> Result {
        let logger = logger.new(o!("module" => "updater"));
        if !self.enabled {
            info!(logger, "disabling");
            return Ok(());
        }
        info!(logger, "starting");
        let mut interval = time::interval(self.interval);
        loop {
            tokio::select! {
                _ = shutdown.clone() => {
                    info!(logger, "shutting down");
                    return Ok(())
                },
                _ = interval.tick() => {
                    // Get the current version and find the first release
                    // version in the settings channel that is newer than the
                    // package version.
                    let current_version = settings::version();
                    let channel = self.channel.clone();
                    let platform = self.platform.clone();
                    match releases::filtered(releases::all(self.uri.to_string()), move | r | {
                        r.in_channel(&channel) && r.version > current_version && r.asset_for_platform(&platform).is_some()
                    }).try_next().await {
                        Ok(Some(release)) => {
                            let asset = release.asset_for_platform(&self.platform).expect("asset for platform");
                            info!(logger, "downloading {asset}", asset = asset.name.clone());
                            let download_path = self.download_path(&asset.name);
                            asset.download(&download_path).await?;
                            info!(logger, "installing {asset}", asset=asset.name.clone());
                            return self.install(&download_path, &logger).await;
                        },
                        Ok(None) => info!(logger,"no update found"),
                        Err(err) => warn!(logger,"failed to fetch releases: {:?}", err),
                    }
                }
            }
        }
    }

    /// Returns a temporary location to download a package into. Do _not_ return a
    /// path that will be used for an actual update since a partial download may
    /// remain after download failures.
    pub fn download_path(&self, package_name: &str) -> PathBuf {
        env::temp_dir().join(package_name)
    }

    /// Does a platform specific install of the given package. Some platform
    /// will remove the package into a staging location and reboot to trigger the
    /// install whereas others may just need a package install and service
    /// restart.
    pub async fn install(&self, download_path: &Path, logger: &Logger) -> Result {
        match process::Command::new(&self.install_command)
            .arg(download_path)
            .output()
            .await
        {
            Ok(output) => {
                if output.status.success() {
                    return Ok(());
                }
                let output = String::from_utf8(output.stderr).unwrap();
                error!(logger, "failed to install update {}", output);
                Err(io::Error::new(io::ErrorKind::Other, output).into())
            }
            Err(err) => Err(err.into()),
        }
    }
}
