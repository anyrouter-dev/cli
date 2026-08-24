//! `anyr upgrade` / `anyr upgrade --check`.
//! Network is skipped when `--fixture` or `ANYR_RELEASES_JSON` is set.

use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;
#[cfg(feature = "native")]
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::channel::{
    current_arch, current_os, release_asset_url, select_latest, Channel, GITHUB_RELEASES_API,
};
use crate::config::{resolve_config_path, write_config, Config};
use crate::http::http_get;
use crate::key::load_config_if_present;
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::redact_value;
use crate::term;
use crate::VERSION;

/// How often the background checker hits GitHub Releases.
pub const DEFAULT_UPDATE_INTERVAL_SECS: u64 = 6 * 60 * 60;

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

/// `--beta` / `--stable` switch the persisted channel. Mutually exclusive with
/// each other and with `--channel`.
fn channel_switch_flag(parsed: &ParsedArgs) -> Result<Option<Channel>, String> {
    let beta = parsed.flag_true("beta");
    let stable = parsed.flag_true("stable");
    if beta && stable {
        return Err("Use either --beta or --stable, not both.".into());
    }
    if (beta || stable) && get_string_flag(&parsed.flags, "channel").is_some() {
        return Err("--beta/--stable cannot be combined with --channel.".into());
    }
    if beta {
        return Ok(Some(Channel::Beta));
    }
    if stable {
        return Ok(Some(Channel::Stable));
    }
    Ok(None)
}

fn resolve_channel(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<Channel, String> {
    if let Some(ch) = channel_switch_flag(parsed)? {
        return Ok(ch);
    }
    if let Some(flag) = get_string_flag(&parsed.flags, "channel") {
        return Channel::parse(&flag);
    }
    if let Some(v) = env
        .get("ANYR_CHANNEL")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return Channel::parse(v);
    }
    let path = resolve_config_path(None, env);
    if let Some(ch) = load_config_if_present(&path).and_then(|c| c.channel) {
        return Channel::parse(&ch);
    }
    Ok(Channel::Stable)
}

/// Persist `channel:` so future auto-updates follow the switched track.
fn persist_channel(channel: Channel, env: &BTreeMap<String, String>) -> Result<bool, String> {
    let path = resolve_config_path(None, env);
    let mut cfg = load_config_if_present(&path).unwrap_or_else(Config::default);
    let next = channel.as_str().to_string();
    let changed = cfg.channel.as_deref() != Some(next.as_str());
    if !changed && cfg.channel.is_some() {
        return Ok(false);
    }
    // Also write when channel was previously unset and we are pinning stable/beta,
    // so subsequent runs don't depend on defaults alone.
    let was_unset = cfg.channel.is_none();
    cfg.channel = Some(next);
    write_config(&cfg, &path)?;
    Ok(changed || was_unset)
}

fn version_eq(a: &str, b: &str) -> bool {
    let a = a.trim().strip_prefix('v').unwrap_or(a.trim());
    let b = b.trim().strip_prefix('v').unwrap_or(b.trim());
    a == b
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

fn env_flag(env: &BTreeMap<String, String>, key: &str) -> Option<bool> {
    let raw = env.get(key)?.trim().to_ascii_lowercase();
    match raw.as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

/// Auto-update is on unless config/env turns it off. `CI` and `ANYR_NO_UPDATE` always skip.
pub fn auto_update_enabled(env: &BTreeMap<String, String>) -> bool {
    if env_flag(env, "ANYR_NO_UPDATE") == Some(true) {
        return false;
    }
    if env_flag(env, "CI") == Some(true) {
        return false;
    }
    if let Some(v) = env_flag(env, "ANYR_AUTO_UPDATE") {
        return v;
    }
    let path = resolve_config_path(None, env);
    load_config_if_present(&path)
        .map(|c| c.auto_update())
        .unwrap_or(true)
}

fn state_dir(env: &BTreeMap<String, String>) -> PathBuf {
    resolve_config_path(None, env)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn update_interval_secs(env: &BTreeMap<String, String>) -> u64 {
    env.get("ANYR_UPDATE_INTERVAL_SECS")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_UPDATE_INTERVAL_SECS)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn stamp_is_fresh(env: &BTreeMap<String, String>) -> bool {
    let interval = update_interval_secs(env);
    if interval == 0 {
        return false;
    }
    let path = state_dir(env).join("update.stamp");
    let Ok(raw) = fs::read_to_string(&path) else {
        return false;
    };
    let Ok(then) = raw.trim().parse::<u64>() else {
        return false;
    };
    now_secs().saturating_sub(then) < interval
}

fn write_stamp(env: &BTreeMap<String, String>) {
    let dir = state_dir(env);
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("update.stamp"), format!("{}\n", now_secs()));
}

fn write_notice(env: &BTreeMap<String, String>, version: &str) {
    let dir = state_dir(env);
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("update.notice"), format!("{version}\n"));
}

struct UpdateLock {
    path: PathBuf,
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn try_lock(env: &BTreeMap<String, String>) -> Option<UpdateLock> {
    let dir = state_dir(env);
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("update.lock");
    if let Ok(meta) = fs::metadata(&path) {
        if let Ok(modified) = meta.modified() {
            if modified
                .elapsed()
                .unwrap_or(Duration::from_secs(0))
                .as_secs()
                > 15 * 60
            {
                let _ = fs::remove_file(&path);
            }
        }
    }
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            let _ = writeln!(file, "{}", std::process::id());
            Some(UpdateLock { path })
        }
        Err(_) => None,
    }
}

fn print_pending_notice(env: &BTreeMap<String, String>) {
    let path = state_dir(env).join("update.notice");
    let Ok(raw) = fs::read_to_string(&path) else {
        return;
    };
    let ver = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
    if ver.is_empty() {
        return;
    }
    let running = VERSION.trim().strip_prefix('v').unwrap_or(VERSION.trim());
    if ver == running {
        eprintln!("anyr: updated to {ver}");
        let _ = fs::remove_file(&path);
    }
}

/// Quiet check+install used by `--auto` and the background worker.
fn run_auto(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let _lock = match try_lock(env) {
        Some(lock) => lock,
        None => return Ok(0),
    };
    if stamp_is_fresh(env) && !parsed.flag_true("force") {
        return Ok(0);
    }
    let channel = resolve_channel(parsed, env)?;
    let fixture = fixture_path(parsed, env);
    let json = match load_releases_json(fixture.as_deref()) {
        Ok(j) => j,
        Err(_) => {
            write_stamp(env);
            return Ok(0);
        }
    };
    let latest = match select_latest(&json, channel) {
        Ok(rel) => rel,
        Err(_) => {
            write_stamp(env);
            return Ok(0);
        }
    };
    let latest_ver = latest.version_str().to_string();
    write_stamp(env);
    if !needs_upgrade(VERSION, &latest_ver) {
        return Ok(0);
    }
    let dry = parsed.flag_true("dry-run") || fixture.is_some();
    if dry {
        println!("would update {VERSION} -> {latest_ver}");
        return Ok(0);
    }
    let url = release_asset_url(&latest, current_os(), current_arch());
    match replace_current_binary(&url) {
        Ok(_) => {
            write_notice(env, &latest_ver);
            println!("updated {VERSION} -> {latest_ver}");
            Ok(0)
        }
        Err(_) => Ok(0),
    }
}

/// Fire-and-forget `anyr upgrade --auto` so short commands still update.
#[cfg(feature = "native")]
fn spawn_detached_auto(env: &BTreeMap<String, String>) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("upgrade")
        .arg("--auto")
        .env("ANYR_AUTO_CHILD", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (k, v) in env {
        if k.starts_with("ANYR") || k == "ANYROUTER_HOME" || k == "HOME" {
            cmd.env(k, v);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    let _ = cmd.spawn();
}

#[cfg(not(feature = "native"))]
fn spawn_detached_auto(_env: &BTreeMap<String, String>) {}

/// Background check on startup. No-op when auto-update is off, already a
/// child, or we checked recently.
pub fn on_startup(command: &str, parsed: &ParsedArgs, env: &BTreeMap<String, String>) {
    let is_help = matches!(command, "help" | "--help" | "-h") || parsed.flag_true("help");
    let is_upgrade = matches!(command, "upgrade" | "update");
    if !is_help && command != "--version" && command != "-v" {
        print_pending_notice(env);
    }
    if is_upgrade || env.get("ANYR_AUTO_CHILD").is_some() {
        return;
    }
    if !auto_update_enabled(env) {
        return;
    }
    if stamp_is_fresh(env) {
        return;
    }
    spawn_detached_auto(env);
}

/// Recheck GitHub while a long-running agent is open (every interval).
#[cfg(feature = "native")]
pub fn start_session_checker(
    env: &BTreeMap<String, String>,
) -> Option<std::thread::JoinHandle<()>> {
    if !auto_update_enabled(env) || env.get("ANYR_AUTO_CHILD").is_some() {
        return None;
    }
    let env = env.clone();
    std::thread::Builder::new()
        .name("anyr-update".into())
        .spawn(move || {
            let parsed = ParsedArgs {
                command: "upgrade".into(),
                flags: std::collections::HashMap::new(),
                passthrough: Vec::new(),
            };
            loop {
                let _ = run_auto(&parsed, &env);
                let sleep_for = update_interval_secs(&env).max(60);
                std::thread::sleep(Duration::from_secs(sleep_for));
            }
        })
        .ok()
}

#[cfg(not(feature = "native"))]
pub fn start_session_checker(_env: &BTreeMap<String, String>) -> Option<()> {
    None
}

pub fn run(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    if parsed.flag_true("auto") {
        return run_auto(parsed, env);
    }
    let switch = channel_switch_flag(parsed)?;
    if let Some(ch) = switch {
        let changed = persist_channel(ch, env)?;
        if changed {
            println!("channel set to {}", ch.as_str());
        }
    }
    let channel = resolve_channel(parsed, env)?;
    let fixture = fixture_path(parsed, env);
    let json = load_releases_json(fixture.as_deref())?;
    let latest = select_latest(&json, channel)?;
    let os = current_os();
    let arch = current_arch();
    let url = release_asset_url(&latest, os, arch);
    let latest_ver = latest.version_str();
    // Channel switches may need a "downgrade" (beta → older stable). Compare
    // equality instead of semver-newer when --beta/--stable was used.
    let update = if switch.is_some() {
        !version_eq(VERSION, latest_ver)
    } else {
        needs_upgrade(VERSION, latest_ver)
    };
    let check = wants_check(parsed);
    let dry = parsed.flag_true("dry-run") || fixture.is_some();

    print_version_report(channel, latest_ver);

    if parsed.flag_true("dry-run") {
        print_redacted_env(env);
    }

    if check {
        println!("{}", field("asset", url));
        if update {
            println!(
                "{}",
                status_change(term::warn("update available"), VERSION, latest_ver)
            );
            println!(
                "{}",
                field("run", format!("{} update", crate::help::invoked_bin()))
            );
        } else {
            println!("{}", field("status", term::ok("up to date")));
        }
        return Ok(0);
    }

    if !update {
        println!("{}", field("status", term::ok("up to date")));
        return Ok(0);
    }

    if dry {
        println!("{}", field("asset", &url));
        println!(
            "{}",
            status_change(term::warn("would update"), VERSION, latest_ver)
        );
        return Ok(0);
    }

    println!("Downloading {url}");
    let dest = replace_current_binary(&url)?;
    println!(
        "{}",
        status_change(term::ok("updated"), VERSION, latest_ver)
    );
    println!("{}", field("path", dest.display()));
    Ok(0)
}

fn field(key: &str, value: impl std::fmt::Display) -> String {
    format!("{:<8} {value}", format!("{key}:"))
}

fn print_version_report(channel: Channel, latest_ver: &str) {
    println!(
        "{}",
        field(
            "current",
            format!("{VERSION} (built {})", crate::buildinfo::display_time())
        )
    );
    println!("{}", field("latest", latest_ver));
    println!("{}", field("channel", channel.as_str()));
}

/// `status:  updated  0.1.0 -> 0.1.1` — `->` is only ever old version to new.
fn status_change(status: impl std::fmt::Display, from: &str, to: &str) -> String {
    field("status", format!("{status}  {from} -> {to}"))
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

    fn isolated_home() -> (BTreeMap<String, String>, PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "anyr-upd-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::create_dir_all(&dir);
        let mut env = BTreeMap::new();
        env.insert("ANYROUTER_HOME".into(), dir.to_string_lossy().into_owned());
        (env, dir)
    }

    #[test]
    fn auto_update_enabled_defaults_on() {
        let (env, _) = isolated_home();
        assert!(auto_update_enabled(&env));
    }

    #[test]
    fn auto_update_disabled_by_env_and_ci() {
        let (mut env, _) = isolated_home();
        env.insert("ANYR_NO_UPDATE".into(), "1".into());
        assert!(!auto_update_enabled(&env));

        let (mut env, _) = isolated_home();
        env.insert("ANYR_AUTO_UPDATE".into(), "0".into());
        assert!(!auto_update_enabled(&env));

        let (mut env, _) = isolated_home();
        env.insert("CI".into(), "true".into());
        assert!(!auto_update_enabled(&env));
    }

    #[test]
    fn auto_update_env_on_overrides_nothing() {
        let (mut env, _) = isolated_home();
        env.insert("ANYR_AUTO_UPDATE".into(), "on".into());
        assert!(auto_update_enabled(&env));
    }

    #[test]
    fn auto_update_config_false_disables() {
        let (env, dir) = isolated_home();
        fs::write(
            dir.join("config.yaml"),
            "active_profile: default\nauto_update: false\nprofiles:\n  default:\n    api_key: x\n",
        )
        .unwrap();
        assert!(!auto_update_enabled(&env));
    }

    fn parsed_check() -> ParsedArgs {
        ParsedArgs {
            command: "upgrade".into(),
            flags: std::collections::HashMap::from([(
                "check".into(),
                crate::parse::FlagValue::Bool(true),
            )]),
            passthrough: Vec::new(),
        }
    }

    #[test]
    fn resolve_channel_reads_config_then_env() {
        let (mut env, dir) = isolated_home();
        fs::write(
            dir.join("config.yaml"),
            "active_profile: default\nchannel: beta\nprofiles:\n  default:\n    api_key: x\n",
        )
        .unwrap();
        assert_eq!(
            resolve_channel(&parsed_check(), &env).unwrap(),
            Channel::Beta
        );
        env.insert("ANYR_CHANNEL".into(), "stable".into());
        assert_eq!(
            resolve_channel(&parsed_check(), &env).unwrap(),
            Channel::Stable
        );
    }

    fn parsed_with(flags: &[(&str, bool)]) -> ParsedArgs {
        ParsedArgs {
            command: "update".into(),
            flags: flags
                .iter()
                .map(|(k, v)| ((*k).into(), crate::parse::FlagValue::Bool(*v)))
                .collect(),
            passthrough: Vec::new(),
        }
    }

    #[test]
    fn beta_stable_flags_resolve_and_conflict() {
        assert_eq!(
            resolve_channel(&parsed_with(&[("beta", true)]), &BTreeMap::new()).unwrap(),
            Channel::Beta
        );
        assert_eq!(
            resolve_channel(&parsed_with(&[("stable", true)]), &BTreeMap::new()).unwrap(),
            Channel::Stable
        );
        let err =
            channel_switch_flag(&parsed_with(&[("beta", true), ("stable", true)])).unwrap_err();
        assert!(err.contains("either --beta or --stable"), "{err}");
    }

    #[test]
    fn beta_flag_conflicts_with_channel() {
        let parsed = ParsedArgs {
            command: "update".into(),
            flags: std::collections::HashMap::from([
                ("beta".into(), crate::parse::FlagValue::Bool(true)),
                (
                    "channel".into(),
                    crate::parse::FlagValue::Value("stable".into()),
                ),
            ]),
            passthrough: Vec::new(),
        };
        let err = channel_switch_flag(&parsed).unwrap_err();
        assert!(err.contains("--channel"), "{err}");
    }

    #[test]
    fn persist_channel_writes_config() {
        let (env, dir) = isolated_home();
        assert!(persist_channel(Channel::Beta, &env).unwrap());
        let raw = fs::read_to_string(dir.join("config.yaml")).unwrap();
        assert!(raw.contains("channel: beta"), "{raw}");
        assert!(!persist_channel(Channel::Beta, &env).unwrap());
        assert!(persist_channel(Channel::Stable, &env).unwrap());
        let raw = fs::read_to_string(dir.join("config.yaml")).unwrap();
        assert!(raw.contains("channel: stable"), "{raw}");
    }

    #[test]
    fn version_eq_ignores_v_prefix() {
        assert!(version_eq("0.1.1", "v0.1.1"));
        assert!(!version_eq("0.1.1", "0.1.2"));
    }

    #[test]
    fn field_aligns_values_and_keeps_colon() {
        assert_eq!(field("current", "0.1.0"), "current: 0.1.0");
        assert_eq!(field("latest", "0.1.1"), "latest:  0.1.1");
        assert_eq!(field("channel", "beta"), "channel: beta");
        assert_eq!(field("status", "up to date"), "status:  up to date");
        assert_eq!(
            field("path", "/home/duyet/.local/bin/anyr"),
            "path:    /home/duyet/.local/bin/anyr"
        );
    }

    #[test]
    fn status_change_is_old_then_new_not_a_path() {
        let line = status_change("updated", "0.1.11-beta.70", "0.1.11-beta.76");
        assert_eq!(line, "status:  updated  0.1.11-beta.70 -> 0.1.11-beta.76");
        assert!(
            !line.contains("/"),
            "status must not reuse -> for an install path: {line}"
        );
    }
}
