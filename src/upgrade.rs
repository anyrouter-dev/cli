//! `anyr upgrade` / `anyr upgrade --check`.
//! Network is skipped when `--fixture` or `ANYR_RELEASES_JSON` is set.

use std::collections::BTreeMap;
use std::fs;
#[cfg(feature = "native")]
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::channel::{
    current_arch, current_os, release_asset_url, select_latest, Channel, GITHUB_RELEASES_API,
};
use crate::http::http_get;
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::redact_value;
use crate::VERSION;

/// True when `latest` is a higher version than `current` (leading `v` ignored).
pub fn needs_upgrade(current: &str, latest: &str) -> bool {
    match (
        crate::channel::parse_version(current),
        crate::channel::parse_version(latest),
    ) {
        (Some(cur), Some(lat)) => lat > cur,
        _ => {
            let cur = current.trim().strip_prefix('v').unwrap_or(current.trim());
            let lat = latest.trim().strip_prefix('v').unwrap_or(latest.trim());
            !lat.is_empty() && lat != cur
        }
    }
}

/// Redact `sk-ar-` values even if the env key name does not look secret.
pub fn redact_printed_value(key: &str, value: &str) -> String {
    if value.contains("sk-ar-") {
        redact_value("API_KEY", value)
    } else {
        redact_value(key, value)
    }
}

pub fn fixture_path(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Option<String> {
    get_string_flag(&parsed.flags, "fixture")
        .or_else(|| env.get("ANYR_RELEASES_JSON").cloned())
        .filter(|s| !s.trim().is_empty())
}

pub fn load_releases_json(fixture: Option<&str>) -> Result<String, String> {
    if let Some(path) = fixture {
        return fs::read_to_string(path)
            .map_err(|e| format!("could not read releases fixture {path}: {e}"));
    }
    let (status, body) = http_get(GITHUB_RELEASES_API, None)?;
    if !(200..300).contains(&status) {
        return Err(format!("GitHub Releases API HTTP {status}"));
    }
    Ok(body)
}

fn resolve_channel(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<Channel, String> {
    if let Some(flag) = get_string_flag(&parsed.flags, "channel") {
        return Channel::parse(&flag);
    }
    Channel::from_env(env)
}

fn wants_check(parsed: &ParsedArgs) -> bool {
    parsed.flag_true("check") || parsed.passthrough.iter().any(|a| a == "check")
}

fn print_redacted_env(env: &BTreeMap<String, String>) {
    println!("env:");
    for key in [
        "ANYR_CHANNEL",
        "ANYR_RELEASES_JSON",
        "ANYROUTER_API_KEY",
        "ANYR_SETUP_BIN",
    ] {
        if let Some(value) = env.get(key) {
            println!("{key}={}", redact_printed_value(key, value));
        }
    }
}

#[cfg(not(feature = "native"))]
fn download_binary(_url: &str, _dest: &Path) -> Result<(), String> {
    Err("download is not available in the browser demo".into())
}

#[cfg(feature = "native")]
fn download_binary(url: &str, dest: &Path) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .user_agent(&format!("anyr-cli/{VERSION}"))
        .build();
    let resp = match agent.get(url).call() {
        Ok(resp) => resp,
        Err(ureq::Error::Status(code, resp)) => {
            let _ = resp.into_string();
            return Err(format!("download HTTP {code} from {url}"));
        }
        Err(err) => return Err(format!("download failed: {err}")),
    };
    if !(200..300).contains(&resp.status()) {
        return Err(format!("download HTTP {} from {url}", resp.status()));
    }
    let mut reader = resp.into_reader();
    let mut file =
        fs::File::create(dest).map_err(|e| format!("could not write {}: {e}", dest.display()))?;
    io::copy(&mut reader, &mut file).map_err(|e| format!("could not save download: {e}"))?;
    file.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn replace_current_binary(url: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let dest = match fs::read_link(&exe) {
        Ok(target) => {
            if target.is_absolute() {
                target
            } else {
                exe.parent().unwrap_or(Path::new(".")).join(target)
            }
        }
        Err(_) => exe,
    };
    let tmp = dest.with_file_name(format!(
        ".{}.new",
        dest.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "anyr".into())
    ));
    download_binary(url, &tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&tmp)
            .map_err(|e| format!("stat {}: {e}", tmp.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tmp, perms).map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
    }
    fs::rename(&tmp, &dest).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("could not replace {}: {e}", dest.display())
    })?;
    Ok(dest)
}

pub fn run(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let channel = resolve_channel(parsed, env)?;
    let fixture = fixture_path(parsed, env);
    let json = load_releases_json(fixture.as_deref())?;
    let latest = select_latest(&json, channel)?;
    let os = current_os();
    let arch = current_arch();
    let url = release_asset_url(&latest, os, arch);
    let latest_ver = latest.version_str();
    let update = needs_upgrade(VERSION, latest_ver);
    let check = wants_check(parsed);
    let dry = parsed.flag_true("dry-run") || fixture.is_some();

    println!("anyr {VERSION}");
    println!("channel: {}", channel.as_str());
    println!("latest: {latest_ver}");
    println!("asset: {url}");

    if parsed.flag_true("dry-run") {
        print_redacted_env(env);
    }

    if check {
        if update {
            println!("update available");
            println!("run: anyr upgrade");
        } else {
            println!("up to date");
        }
        return Ok(0);
    }

    if !update {
        println!("Already up to date ({VERSION}).");
        return Ok(0);
    }

    if dry {
        println!("Would upgrade {VERSION} -> {latest_ver}");
        println!("Would download {url}");
        return Ok(0);
    }

    println!("Upgrading {VERSION} -> {latest_ver}");
    println!("Downloading {url}");
    let dest = replace_current_binary(&url)?;
    println!("Installed {latest_ver} -> {}", dest.display());
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{release_asset_url, select_latest, Channel, GITHUB_DOWNLOAD_PREFIX};

    const FIXTURE: &str = r#"[
  {"tag_name":"v0.1.1","prerelease":false,"assets":[{"name":"anyr-linux-x86_64","browser_download_url":"https://github.com/anyrouter-dev/cli/releases/download/v0.1.1/anyr-linux-x86_64"}]},
  {"tag_name":"v0.1.2-beta.1","prerelease":true,"assets":[{"name":"anyr-linux-x86_64","browser_download_url":"https://github.com/anyrouter-dev/cli/releases/download/v0.1.2-beta.1/anyr-linux-x86_64"}]}
]"#;

    #[test]
    fn select_latest_stable_skips_beta() {
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
    fn needs_upgrade_newer_is_true() {
        assert!(needs_upgrade("0.1.0", "0.1.1"));
    }

    #[test]
    fn needs_upgrade_same_is_false() {
        assert!(!needs_upgrade("0.1.1", "0.1.1"));
        assert!(!needs_upgrade("0.1.1", "v0.1.1"));
    }

    #[test]
    fn release_asset_url_uses_github_releases_download() {
        let rel = select_latest(FIXTURE, Channel::Stable).unwrap();
        let url = release_asset_url(&rel, "linux", "x86_64");
        assert!(
            url.contains("github.com/anyrouter-dev/cli/releases/download/"),
            "{url}"
        );
        assert!(url.contains(GITHUB_DOWNLOAD_PREFIX), "{url}");
        assert!(url.ends_with("/anyr-linux-x86_64"), "{url}");
    }

    #[test]
    fn redact_sk_ar_keys_in_dry_run_env() {
        let redacted = redact_printed_value("ANYROUTER_API_KEY", "sk-ar-v1-secret-value");
        assert!(!redacted.contains("sk-ar-v1-secret-value"));
        assert!(redacted.contains("sk-ar-"));
    }
}
