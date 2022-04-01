use crate::{curl, releases, settings, Error, Future, Result, Stream};
use futures::{future, stream, FutureExt, StreamExt, TryFutureExt};
use semver::{Identifier, Version};
use serde::{de, Deserialize, Deserializer};
use std::{fmt, path::Path, str::FromStr};
use tokio::process;

pub const GH_PAGE_SIZE: u8 = 10;

/// Filter a given stream of releases with a given filter function.
pub fn filtered<F>(releases: Stream<Release>, filter: F) -> Stream<Release>
where
    F: Fn(&Release) -> bool + Sync + 'static + std::marker::Send,
{
    releases
        .filter(move |release| future::ready(release.as_ref().map_or(false, &filter)))
        .boxed()
}

/// Get a stream of all releases
pub fn all(url: String) -> Stream<Release> {
    fetch_releases(url, 1)
        .map_ok(move |((url, page), items)| {
            stream::try_unfold(
                ((url, page), items),
                |((url, page), mut items)| async move {
                    match items.pop() {
                        Some(item) => Ok(Some((item, ((url, page), items)))),
                        None => {
                            let ((url, page), mut items) = fetch_releases(url, page + 1).await?;
                            match items.pop() {
                                Some(item) => Ok(Some((item, ((url, page), items)))),
                                None => Ok(None),
                            }
                        }
                    }
                },
            )
        })
        .try_flatten_stream()
        .boxed()
}

fn fetch_releases(url: String, page: u32) -> Future<((String, u32), Vec<Release>)> {
    let curl_url = format!("{url}?per_page={GH_PAGE_SIZE}&page={page}");
    curl::get(
        curl_url,
        &["-H", "Accept: application/vnd.github.v3+json"],
        move |output| {
            let mut items: Vec<Release> = serde_json::from_slice(output)?;
            items.reverse();
            Ok(((url, page), items))
        },
    )
    .boxed()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelParseError(String);

impl fmt::Display for ChannelParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid channel: {}", self.0)
    }
}

/// Represents a release channel
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Channel {
    Alpha,
    Beta,
    Release,
}

impl<'de> Deserialize<'de> for Channel {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ChannelVisitor;

        impl<'de> de::Visitor<'de> for ChannelVisitor {
            type Value = Channel;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("release channel")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Channel, E>
            where
                E: de::Error,
            {
                let value = value.to_lowercase();
                match value.parse::<releases::Channel>() {
                    Ok(channel) => Ok(channel),
                    Err(_) => {
                        return Err(de::Error::custom(format!(
                            "unsupported update channel: \"{value}\"",
                        )))
                    }
                }
            }
        }

        deserializer.deserialize_str(ChannelVisitor)
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Channel::Alpha => "alpha",
            Channel::Beta => "beta",
            Channel::Release => "release",
        };
        f.write_str(s)
    }
}

impl FromStr for Channel {
    type Err = ChannelParseError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "alpha" => Ok(Channel::Alpha),
            "beta" => Ok(Channel::Beta),
            "release" => Ok(Channel::Release),
            "semver" => Ok(Self::from_version(&settings::version())),
            invalid => Err(ChannelParseError(invalid.to_string())),
        }
    }
}

impl Channel {
    pub fn from_version(version: &Version) -> Self {
        for identifier in &version.pre {
            if let Identifier::AlphaNumeric(v) = identifier {
                match v.as_str() {
                    "alpha" => return Channel::Alpha,
                    "beta" => return Channel::Beta,
                    _ => continue,
                }
            }
        }
        Channel::Release
    }
}

/// Represents a versioned release  with one or more assets
#[derive(Debug, Deserialize)]
pub struct Release {
    /// The version of the release
    #[serde(deserialize_with = "deserialize_version", rename = "tag_name")]
    pub version: Version,
    /// The list of assets for the release
    pub assets: Vec<ReleaseAsset>,
}

fn deserialize_version<'de, D>(d: D) -> std::result::Result<Version, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    let version_str = match s.strip_prefix('v') {
        Some(v) => v,
        None => &s,
    };
    version_str
        .parse()
        .map_err(|e| de::Error::custom(format!("invalid release format \"{s}\": {e}")))
}

impl Release {
    /// Checks whether a release is in the given channel. For the release
    /// channel any non prerelease version is considered good. For alpha/beta
    /// the alpha or beta strings have to be part of the "pre" release
    /// identifiers of the version.
    pub fn in_channel(&self, channel: &Channel) -> bool {
        match channel {
            Channel::Release => !self.version.is_prerelease(),
            Channel::Alpha | Channel::Beta => {
                let tag = channel.to_string();
                for identifier in &self.version.pre {
                    if let Identifier::AlphaNumeric(v) = identifier {
                        if v.contains(&tag) {
                            return true;
                        }
                    }
                }
                false
            }
        }
    }

    pub fn asset_for_platform(&self, platform: &str) -> Option<&ReleaseAsset> {
        let version = &self.version;
        let package_name = format!("helium-gateway-v{version}-{platform}");
        self.asset_named(&package_name)
    }

    /// Find an asset with a given name in this release. Returns None if no such
    /// asset was found.
    pub fn asset_named(&self, name: &str) -> Option<&ReleaseAsset> {
        for asset in &self.assets {
            if asset.name.starts_with(name) {
                return Some(asset);
            }
        }
        None
    }
}

/// A release asset is a named, downloadable file that can be installed on a
/// system.
#[derive(Debug, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub download_url: String,
    pub size: usize,
}

impl ReleaseAsset {
    /// Downloads the asset to a given destination.
    pub async fn download(&self, dest: &Path) -> Result {
        process::Command::new("curl")
            .kill_on_drop(true)
            .arg("-s")
            .arg("-L")
            .args(&["-o", &dest.to_string_lossy()])
            .arg(&self.download_url)
            .status()
            .map(|status| match status {
                Ok(exit_status) if exit_status.success() => Ok(()),
                Ok(exit_status) => Err(Error::custom(format!(
                    "failed to download asset {}: {:?}",
                    self.download_url,
                    exit_status.code()
                ))),
                Err(err) => Err(Error::from(err)),
            })
            .await
    }
}
