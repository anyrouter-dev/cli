//! Background self-update against a mock release server.
//!
//! Each test copies the built `anyr` into a temp "install" dir (a binary under
//! cargo `target/` refuses to self-update) and points it at a loopback server
//! through `ANYR_UPDATE_BASE_URL`.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyr_cli::channel::{asset_name, current_arch, current_os};
use sha2::{Digest, Sha256};

const NEW_BUILD: &[u8] = b"anyr build 999.0.0 from the mock release server";

fn asset() -> String {
    asset_name(current_os(), current_arch())
}

fn sha_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct MockRelease {
    base: String,
    hits: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct Release {
    tag: &'static str,
    prerelease: bool,
    /// None: the asset returns HTTP 500.
    body: Option<Vec<u8>>,
    /// Checksum line written for the asset; defaults to the real hash.
    checksum: Option<String>,
}

impl Release {
    fn good(tag: &'static str, prerelease: bool) -> Self {
        Self {
            tag,
            prerelease,
            body: Some(NEW_BUILD.to_vec()),
            checksum: None,
        }
    }
}

/// Serves `/releases.json` (after `listing_delay`), each asset and each
/// release's `checksums.txt`.
fn serve(releases: Vec<Release>, listing_delay: Duration) -> MockRelease {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let (b, h) = (base.clone(), Arc::clone(&hits));
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            h.fetch_add(1, Ordering::SeqCst);
            let (base, releases) = (b.clone(), releases.clone());
            thread::spawn(move || {
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, body) = route(&path, &base, &releases, listing_delay);
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            });
        }
    });
    MockRelease { base, hits }
}

fn route(path: &str, base: &str, releases: &[Release], delay: Duration) -> (u16, Vec<u8>) {
    let asset = asset();
    if path == "/releases.json" {
        thread::sleep(delay);
        let list: Vec<serde_json::Value> = releases
            .iter()
            .map(|r| {
                serde_json::json!({
                    "tag_name": r.tag,
                    "prerelease": r.prerelease,
                    "assets": [{
                        "name": asset,
                        "browser_download_url":
                            format!("{base}/releases/download/{}/{asset}", r.tag),
                    }],
                })
            })
            .collect();
        return (200, serde_json::Value::Array(list).to_string().into_bytes());
    }
    for r in releases {
        let dir = format!("/releases/download/{}/", r.tag);
        let Some(file) = path.strip_prefix(&dir) else {
            continue;
        };
        if file == "checksums.txt" {
            let sum = r
                .checksum
                .clone()
                .unwrap_or_else(|| sha_hex(r.body.as_deref().unwrap_or_default()));
            return (200, format!("{sum}  {asset}\n").into_bytes());
        }
        if file == asset {
            return match &r.body {
                Some(b) => (200, b.clone()),
                None => (500, b"boom".to_vec()),
            };
        }
    }
    (404, b"not found".to_vec())
}

struct Install {
    root: PathBuf,
    bin: PathBuf,
    original: Vec<u8>,
}

impl Install {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "anyr-self-update-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        let name = if cfg!(windows) { "anyr.exe" } else { "anyr" };
        let bin = root.join("bin").join(name);
        fs::copy(env!("CARGO_BIN_EXE_anyr"), &bin).unwrap();
        let original = fs::read(&bin).unwrap();
        Self {
            root,
            bin,
            original,
        }
    }

    fn sibling(&self, kind: &str) -> PathBuf {
        let name = self.bin.file_name().unwrap().to_string_lossy();
        self.bin.with_file_name(format!(".{name}.{kind}"))
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn state(&self) -> serde_json::Value {
        fs::read_to_string(self.home().join("update-state.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::Value::Null)
    }

    fn write_config(&self, body: &str) {
        fs::write(self.home().join("config.yaml"), body).unwrap();
    }

    /// Run the installed copy. Auto-update is allowed off a TTY (tests have
    /// none) unless `extra` turns it off.
    fn run(&self, mock: &MockRelease, args: &[&str], extra: &[(&str, &str)]) -> Output {
        let mut env: BTreeMap<&str, &str> = BTreeMap::from([
            ("ANYROUTER_HOME", self.home().to_str().unwrap()),
            ("ANYR_UPDATE_BASE_URL", mock.base.as_str()),
            ("ANYR_AUTO_UPDATE", "1"),
            ("NO_COLOR", "1"),
        ]);
        for (k, v) in extra {
            env.insert(k, v);
        }
        let mut cmd = Command::new(&self.bin);
        for k in [
            "CI",
            "ANYR_NO_UPDATE",
            "ANYR_NO_AUTO_UPDATE",
            "ANYR_CHANNEL",
            "ANYR_OUTPUT",
            "ANYR_QUIET",
            "ANYR_LAUNCHER_CHILD",
            "ANYR_AUTO_CHILD",
            "ANYR_RELAY_DETACHED",
            "ANYR_UPDATE_INTERVAL_SECS",
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "ANYR_GITHUB_TOKEN",
        ] {
            cmd.env_remove(k);
        }
        cmd.envs(env)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap()
    }

    /// Wait for the detached checker to finish (it records `last_check`).
    fn wait_checked(&self) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let state = self.state();
            let done = state["last_check"].as_u64().unwrap_or(0) > 0
                && !self.home().join("update.lock").exists();
            if done {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "background check never finished: {state}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Install {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn check_never_blocks_the_command() {
    // WHY: a slow GitHub must never slow `anyr`; the listing here takes 3s.
    let mock = serve(
        vec![Release::good("v999.0.0", false)],
        Duration::from_secs(3),
    );
    let install = Install::new();
    let started = Instant::now();
    let out = install.run(&mock, &["config"], &[]);
    let took = started.elapsed();
    assert!(
        took < Duration::from_millis(1500),
        "command waited {took:?} on the update check; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The detached check still runs to completion afterwards.
    let state = install.wait_checked();
    assert_eq!(state["pending_version"], "999.0.0", "{state}");
}

#[test]
fn pending_update_applies_on_the_next_run_and_rollback_restores() {
    let mock = serve(vec![Release::good("v999.0.0", false)], Duration::ZERO);
    let install = Install::new();

    install.run(&mock, &["config"], &[]);
    let state = install.wait_checked();
    assert_eq!(state["pending_version"], "999.0.0", "{state}");
    assert_eq!(
        fs::read(&install.bin).unwrap(),
        install.original,
        "the check must stage, not swap, the binary"
    );
    assert_eq!(fs::read(install.sibling("pending")).unwrap(), NEW_BUILD);

    // Next run: the staged build replaces the binary, the old one is kept.
    let out = install.run(&mock, &["config"], &[]);
    assert_eq!(fs::read(&install.bin).unwrap(), NEW_BUILD);
    assert_eq!(fs::read(install.sibling("prev")).unwrap(), install.original);
    assert!(!install.sibling("pending").exists());
    assert!(install.state()["pending_path"].is_null());
    // Off a TTY the "anyr updated" line stays out of the output.
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("anyr updated"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The swapped-in build is a stub that cannot run, so put the real anyr
    // back as the binary with the stub as `prev`, then roll back to the stub.
    let prev = install.sibling("prev");
    fs::rename(&prev, install.root.join("prev-copy")).unwrap();
    fs::write(&prev, NEW_BUILD).unwrap();
    fs::rename(&install.bin, install.root.join("stub")).unwrap();
    fs::rename(install.root.join("prev-copy"), &install.bin).unwrap();
    let out = install.run(&mock, &["update", "--rollback"], &[]);
    assert!(
        out.status.success(),
        "rollback failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(&install.bin).unwrap(), NEW_BUILD);
    assert_eq!(fs::read(&prev).unwrap(), install.original);
    assert_eq!(
        install.state()["skip_version"],
        env!("CARGO_PKG_VERSION"),
        "auto-update must not reinstall the version just rolled back from"
    );
}

#[test]
fn checksum_mismatch_is_rejected_and_recorded() {
    let mut bad = Release::good("v999.0.0", false);
    bad.checksum = Some("0".repeat(64));
    let mock = serve(vec![bad], Duration::ZERO);
    let install = Install::new();
    install.run(&mock, &["config"], &[]);
    let state = install.wait_checked();
    assert!(state["pending_path"].is_null(), "{state}");
    assert!(
        state["last_error"]
            .as_str()
            .unwrap_or_default()
            .contains("checksum mismatch"),
        "{state}"
    );
    assert!(!install.sibling("pending").exists());
    assert!(!install.sibling("download").exists());
    assert_eq!(fs::read(&install.bin).unwrap(), install.original);
}

#[test]
fn failed_download_leaves_the_binary_intact() {
    let mut broken = Release::good("v999.0.0", false);
    broken.body = None;
    let mock = serve(vec![broken], Duration::ZERO);
    let install = Install::new();
    let out = install.run(&mock, &["config"], &[]);
    let state = install.wait_checked();
    assert!(state["last_error"].as_str().is_some(), "{state}");
    assert!(state["pending_path"].is_null(), "{state}");
    assert!(!install.sibling("download").exists());
    assert_eq!(fs::read(&install.bin).unwrap(), install.original);
    // Silent: nothing about the failure on the command's own output.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("download"), "{stderr}");
    // `update --status --json` reports it.
    let status = install.run(&mock, &["update", "--status", "--json"], &[]);
    let body: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(body["last_error"].as_str().is_some(), "{body}");
    assert_eq!(body["current"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn opt_outs_never_touch_the_network() {
    // WHY: no update traffic or swap in CI, for agents parsing --json, or
    // when the user turned it off.
    let cases: &[(&[&str], &[(&str, &str)])] = &[
        (&["config"], &[("CI", "true")]),
        (&["config"], &[("ANYR_NO_AUTO_UPDATE", "1")]),
        (&["config", "--json"], &[]),
        (&["config"], &[("ANYR_OUTPUT", "json")]),
        (&["config"], &[("ANYR_LAUNCHER_CHILD", "1")]),
    ];
    for (args, extra) in cases {
        let mock = serve(vec![Release::good("v999.0.0", false)], Duration::ZERO);
        let install = Install::new();
        install.run(&mock, args, extra);
        thread::sleep(Duration::from_millis(1500));
        assert_eq!(
            mock.hits.load(Ordering::SeqCst),
            0,
            "{args:?} {extra:?} reached the release server"
        );
        assert!(install.state().is_null(), "{args:?} {extra:?}");
    }
}

#[test]
fn stable_install_ignores_beta_and_never_downgrades() {
    let mock = serve(
        vec![
            Release::good("v999.0.0-beta.1", true),
            Release::good("v0.0.1", false),
        ],
        Duration::ZERO,
    );
    let install = Install::new();
    install.write_config("active_profile: default\nchannel: stable\n");
    install.run(&mock, &["config"], &[]);
    let state = install.wait_checked();
    assert!(state["pending_path"].is_null(), "{state}");
    assert!(state["latest_seen"].is_null(), "{state}");
    assert_eq!(state["channel"], "stable");
    assert_eq!(fs::read(&install.bin).unwrap(), install.original);
}

#[test]
fn beta_install_stays_on_beta() {
    let mock = serve(
        vec![
            Release::good("v999.0.0-beta.1", true),
            Release::good("v0.0.1", false),
        ],
        Duration::ZERO,
    );
    let install = Install::new();
    install.write_config("active_profile: default\nchannel: beta\n");
    install.run(&mock, &["config"], &[]);
    let state = install.wait_checked();
    assert_eq!(state["pending_version"], "999.0.0-beta.1", "{state}");
    assert_eq!(state["channel"], "beta");
}

#[cfg(unix)]
#[test]
fn read_only_install_gets_a_hint_and_no_download() {
    use std::os::unix::fs::PermissionsExt;
    let mock = serve(vec![Release::good("v999.0.0", false)], Duration::ZERO);
    let install = Install::new();
    let dir = install.bin.parent().unwrap().to_path_buf();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    if fs::File::create(dir.join("probe")).is_ok() {
        // Running as root: permissions are not enforced, nothing to test.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    install.run(&mock, &["config"], &[]);
    let state = install.wait_checked();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(state["pending_path"].is_null(), "{state}");
    assert!(
        state["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("update"),
        "{state}"
    );
    assert!(
        state["last_error"]
            .as_str()
            .unwrap_or_default()
            .contains("not writable"),
        "{state}"
    );
    assert_eq!(fs::read(&install.bin).unwrap(), install.original);
}
