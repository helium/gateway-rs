use crate::*;
use futures::{StreamExt, TryStreamExt};
use releases::{self, Channel};
use std::{env, path::PathBuf};
use structopt::StructOpt;

/// Commands for gateway updates
#[derive(Debug, StructOpt)]
pub enum Cmd {
    List(List),
    Download(Download),
}

/// List available updates.
#[derive(Debug, StructOpt)]
pub struct List {
    /// Channel to list updates for (defaults to 'update.channel' setting)
    #[structopt(long)]
    channel: Option<Channel>,
    /// Number of entries to list (default 10)
    #[structopt(short = "n")]
    count: Option<usize>,
    /// Platform to list entries for (defaults to 'update.platform' setting)
    #[structopt(long)]
    platform: Option<String>,
}

/// Download an updates. This does not install the update
#[derive(Debug, StructOpt)]
pub struct Download {
    // Version of the app to download
    version: semver::Version,
    /// Path to download update to (defaults to current directory)
    #[structopt(long)]
    path: Option<PathBuf>,
}

impl Cmd {
    pub async fn run(&self, settings: Settings) -> Result {
        match self {
            Cmd::List(cmd) => cmd.run(settings).await,
            Cmd::Download(cmd) => cmd.run(settings).await,
        }
    }
}

impl List {
    pub async fn run(&self, settings: Settings) -> Result {
        let channel = self.channel.clone().unwrap_or(settings.update.channel);
        let platform = self.platform.clone().unwrap_or(settings.update.platform);
        let mut releases =
            releases::filtered(releases::all(settings.update.url.to_string()), move |r| {
                r.in_channel(&channel) && r.asset_for_platform(&platform).is_some()
            })
            .take(self.count.unwrap_or(10));
        while let Some(Ok(release)) = releases.next().await {
            if settings::version() == release.version {
                println!("{} (*)", release.version);
            } else {
                println!("{}", release.version);
            }
        }
        Ok(())
    }
}

impl Download {
    pub async fn run(&self, settings: Settings) -> Result {
        let platform = settings.update.platform.clone();
        let version = self.version.clone();
        let channel = Channel::from_version(&version);
        let mut releases =
            releases::filtered(releases::all(settings.update.url.to_string()), move |r| {
                r.version == version
                    && r.in_channel(&channel)
                    && r.asset_for_platform(&platform).is_some()
            });
        let platform = settings.update.platform.clone();
        match releases.try_next().await {
            Ok(Some(release)) => {
                let asset = release
                    .asset_for_platform(&platform)
                    .expect("release asset");
                let download_path = self
                    .path
                    .as_ref()
                    .unwrap_or(&env::current_dir()?)
                    .join(&asset.name);
                match asset.download(&download_path).await {
                    Ok(()) => println!("Downloaded to: {}", download_path.to_string_lossy()),
                    Err(err) => eprintln!("Failed to download update: {:?}", err),
                }
            }
            Ok(None) => eprintln!("No release found"),
            Err(err) => eprintln!("Error finding release: {:?}", err),
        }
        Ok(())
    }
}
