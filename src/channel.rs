//! GitHub Release channels (stable / beta) and asset URL helpers.
//! Pure: parse fixture JSON, pick a release, build download URLs. No network.

use std::cmp::Ordering;

pub const GITHUB_REPO: &str = "anyrouter-dev/cli";
pub const GITHUB_RELEASES_API: &str = "https://api.github.com/repos/anyrouter-dev/cli/releases";
pub const GITHUB_DOWNLOAD_PREFIX: &str = "https://github.com/anyrouter-dev/cli/releases/download/";
pub const GITHUB_LATEST_DOWNLOAD: &str =
    "https://github.com/anyrouter-dev/cli/releases/latest/download";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Beta,
}

impl Channel {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "stable" | "latest" => Ok(Channel::Stable),
            "beta" | "pre" | "prerelease" => Ok(Channel::Beta),
            other => Err(format!("Unknown channel \"{other}\". Use stable or beta.")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
        }
    }

    pub fn from_env(env: &std::collections::BTreeMap<String, String>) -> Result<Self, String> {
        match env.get("ANYR_CHANNEL") {
            Some(v) if !v.trim().is_empty() => Channel::parse(v),
            _ => Ok(Channel::Stable),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag_name: String,
    pub prerelease: bool,
    pub assets: Vec<ReleaseAsset>,
}

impl Release {
    pub fn version_str(&self) -> &str {
        self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Option<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        parse_version(s)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)) {
            Ordering::Equal => match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => cmp_prerelease(a, b),
            },
            other => other,
        }
    }
}

fn cmp_prerelease(a: &str, b: &str) -> Ordering {
    let a_parts: Vec<&str> = a.split('.').collect();
    let b_parts: Vec<&str> = b.split('.').collect();
    let n = a_parts.len().max(b_parts.len());
    for i in 0..n {
        match (a_parts.get(i), b_parts.get(i)) {
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(xn), Ok(yn)) => xn.cmp(&yn),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => (*x).cmp(*y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (None, None) => break,
        }
    }
    Ordering::Equal
}

pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim().strip_prefix('v').unwrap_or(s.trim());
    if s.is_empty() {
        return None;
    }
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, Some(p.to_string())),
        None => (s, None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some(Version {
        major,
        minor,
        patch,
        prerelease: pre.filter(|p| !p.is_empty()),
    })
}

pub fn current_os() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

pub fn current_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    }
}

pub fn asset_name(os: &str, arch: &str) -> String {
    format!("anyr-{os}-{arch}")
}

pub fn parse_releases(json: &str) -> Result<Vec<Release>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("invalid releases JSON: {e}"))?;
    let items: Vec<serde_json::Value> = match value {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(_) => vec![value],
        _ => return Err("releases JSON must be an array or object".into()),
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if item.get("draft").and_then(|d| d.as_bool()) == Some(true) {
            continue;
        }
        let Some(tag) = item.get("tag_name").and_then(|t| t.as_str()) else {
            continue;
        };
        let prerelease = item
            .get("prerelease")
            .and_then(|p| p.as_bool())
            .unwrap_or(false);
        let assets = item
            .get("assets")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        let name = a.get("name")?.as_str()?.to_string();
                        let url = a.get("browser_download_url")?.as_str()?.to_string();
                        Some(ReleaseAsset {
                            name,
                            browser_download_url: url,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.push(Release {
            tag_name: tag.to_string(),
            prerelease,
            assets,
        });
    }
    Ok(out)
}

/// Latest release for `channel` from a GitHub `/releases` JSON array.
/// Stable ignores prereleases; beta picks the highest prerelease tag.
pub fn select_latest(json: &str, channel: Channel) -> Result<Release, String> {
    let releases = parse_releases(json)?;
    select_latest_release(&releases, channel)
}

pub fn select_latest_release(releases: &[Release], channel: Channel) -> Result<Release, String> {
    let mut best: Option<(&Release, Version)> = None;
    for rel in releases {
        let matches = match channel {
            Channel::Stable => !rel.prerelease,
            Channel::Beta => rel.prerelease,
        };
        if !matches {
            continue;
        }
        let Some(ver) = parse_version(&rel.tag_name) else {
            continue;
        };
        let take = match &best {
            None => true,
            Some((_, cur)) => ver > *cur,
        };
        if take {
            best = Some((rel, ver));
        }
    }
    best.map(|(r, _)| r.clone()).ok_or_else(|| match channel {
        Channel::Stable => "no stable release found".into(),
        Channel::Beta => "no beta (prerelease) found".into(),
    })
}

/// Download URL for `anyr-{os}-{arch}` on this release.
/// Prefers `browser_download_url` from the asset list; otherwise constructs
/// `https://github.com/anyrouter-dev/cli/releases/download/{tag}/{asset}`.
pub fn release_asset_url(release: &Release, os: &str, arch: &str) -> String {
    let name = asset_name(os, arch);
    if let Some(asset) = release.assets.iter().find(|a| a.name == name) {
        if !asset.browser_download_url.is_empty() {
            return asset.browser_download_url.clone();
        }
    }
    let tag = if release.tag_name.starts_with('v') {
        release.tag_name.clone()
    } else {
        format!("v{}", release.tag_name)
    };
    format!("{GITHUB_DOWNLOAD_PREFIX}{tag}/{name}")
}

pub fn latest_stable_download_url(os: &str, arch: &str) -> String {
    format!("{GITHUB_LATEST_DOWNLOAD}/{}", asset_name(os, arch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_parse_stable_and_beta() {
        assert_eq!(Channel::parse("stable").unwrap(), Channel::Stable);
        assert_eq!(Channel::parse("BETA").unwrap(), Channel::Beta);
        assert!(Channel::parse("nightly").is_err());
    }

    #[test]
    fn version_prerelease_is_less_than_release() {
        let rel = parse_version("0.1.1").unwrap();
        let pre = parse_version("v0.1.1-beta.1").unwrap();
        assert!(pre < rel);
        assert!(parse_version("0.1.2-beta.1").unwrap() > rel);
    }

    const FIXTURE: &str = include_str!("../tests/fixtures/releases.json");

    #[test]
    fn select_latest_stable_skips_prerelease() {
        let rel = select_latest(FIXTURE, Channel::Stable).unwrap();
        assert_eq!(rel.version_str(), "0.1.1");
        assert!(!rel.prerelease);
    }

    #[test]
    fn select_latest_beta_picks_prerelease() {
        let rel = select_latest(FIXTURE, Channel::Beta).unwrap();
        assert_eq!(rel.version_str(), "0.1.2-beta.1");
        assert!(rel.prerelease);
    }

    #[test]
    fn latest_stable_download_url_is_public_github() {
        let url = latest_stable_download_url("linux", "x86_64");
        assert_eq!(
            url,
            "https://github.com/anyrouter-dev/cli/releases/latest/download/anyr-linux-x86_64"
        );
        assert!(!url.contains("duyet/anyrouter"));
    }

    #[test]
    fn from_env_defaults_stable() {
        let env = std::collections::BTreeMap::new();
        assert_eq!(Channel::from_env(&env).unwrap(), Channel::Stable);
        let mut env = std::collections::BTreeMap::new();
        env.insert("ANYR_CHANNEL".into(), "beta".into());
        assert_eq!(Channel::from_env(&env).unwrap(), Channel::Beta);
    }
}
