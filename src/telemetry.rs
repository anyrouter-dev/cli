//! Anonymous usage ping: at most one per machine per UTC day, sent by a
//! detached `anyr __telemetry <json>` child so the command never waits on it.
//!
//! Sent: event name, random install id, version, OS, CPU arch, CI flag and
//! the subcommand name. Never sent: arguments, paths, hostnames, keys.
//! Opt out with `DO_NOT_TRACK=1`, `ANYR_TELEMETRY_DISABLED=1`, `telemetry:
//! false` in the config file, or an empty `ANYR_TELEMETRY_ENDPOINT`.

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::channel::{current_arch, current_os};
use crate::cmd::dispatch::{canonical_command, known_command};
use crate::config::resolve_config_path;
use crate::key::load_config_if_present;
use crate::upgrade::{env_flag, now_secs, running_from_cargo_target, state_dir};
use crate::VERSION;

/// Where pings go. Override with `ANYR_TELEMETRY_ENDPOINT` (empty disables).
pub const ENDPOINT: &str = "https://telemetry.anyr.workers.dev/anyrouter-cli";

const TIMEOUT: Duration = Duration::from_secs(1);
const ID_FILE: &str = "telemetry-id";
const STAMP_FILE: &str = "telemetry.stamp";
const SECS_PER_DAY: u64 = 24 * 60 * 60;

fn notice() -> String {
    format!(
        "anyr collects anonymous usage data. Opt out: ANYR_TELEMETRY_DISABLED=1 or \
         DO_NOT_TRACK=1. Details: {ENDPOINT}"
    )
}

/// What `on_startup` should do this run.
#[derive(Debug, PartialEq)]
enum Action {
    Skip,
    /// Print the payload, send nothing (`ANYR_TELEMETRY_DEBUG=1`).
    Debug(String),
    /// Hand the payload to the detached sender.
    Send(String),
}

fn endpoint(env: &BTreeMap<String, String>) -> Option<String> {
    match env.get("ANYR_TELEMETRY_ENDPOINT") {
        Some(v) => Some(v.trim().to_string()).filter(|v| !v.is_empty()),
        None => Some(ENDPOINT.to_string()),
    }
}

/// `CI` set to anything but empty / `0` / `false`.
fn in_ci(env: &BTreeMap<String, String>) -> bool {
    env.get("CI").is_some_and(|v| {
        let v = v.trim().to_ascii_lowercase();
        !matches!(v.as_str(), "" | "0" | "false")
    })
}

fn enabled(env: &BTreeMap<String, String>) -> bool {
    if env_flag(env, "DO_NOT_TRACK") == Some(true)
        || env_flag(env, "ANYR_TELEMETRY_DISABLED") == Some(true)
        || in_ci(env)
        || endpoint(env).is_none()
    {
        return false;
    }
    let path = resolve_config_path(None, env);
    load_config_if_present(&path)
        .and_then(|c| c.telemetry)
        .unwrap_or(true)
}

/// Subcommand name from a fixed list. Anything the user typed that is not a
/// real command becomes `unknown`, so no free-form input leaves the machine.
fn command_label(command: &str) -> String {
    match command {
        "help" | "--help" | "-h" => "help".into(),
        c if known_command(c) => canonical_command(c).into(),
        _ => "unknown".into(),
    }
}

fn payload(id: Option<&str>, command: &str, ci: bool) -> Value {
    let mut body = Map::new();
    body.insert("event".into(), json!("ping"));
    if let Some(id) = id {
        body.insert("id".into(), json!(id));
    }
    body.insert("version".into(), json!(VERSION));
    body.insert("os".into(), json!(current_os()));
    body.insert("arch".into(), json!(current_arch()));
    body.insert("ci".into(), json!(ci));
    body.insert("props".into(), json!({ "command": command_label(command) }));
    Value::Object(body)
}

/// Random UUID v4. `RandomState` is seeded from OS entropy, which avoids a
/// new dependency for a value that only has to be unique, not secret.
fn random_uuid() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut b = [0u8; 16];
    for half in b.chunks_mut(8) {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(nanos);
        hasher.write_u32(std::process::id());
        half.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn is_valid_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// The persisted install id, created on first use. The bool is true when the
/// id is new, which is when the first-run notice is shown. When the id cannot
/// be saved the ping goes out without one (it then counts daily users only).
fn load_or_create_id(env: &BTreeMap<String, String>) -> (Option<String>, bool) {
    let dir = state_dir(env);
    let path = dir.join(ID_FILE);
    if let Ok(raw) = fs::read_to_string(&path) {
        let id = raw.trim();
        if is_valid_id(id) {
            return (Some(id.to_string()), false);
        }
    }
    let id = random_uuid();
    let saved = crate::config::ensure_private_dir(&dir)
        .and_then(|_| crate::config::write_private(&path, format!("{id}\n").as_bytes()))
        .is_ok();
    (saved.then_some(id), true)
}

fn today() -> u64 {
    now_secs() / SECS_PER_DAY
}

fn sent_today(env: &BTreeMap<String, String>, today: u64) -> bool {
    fs::read_to_string(state_dir(env).join(STAMP_FILE))
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .is_some_and(|day| day >= today)
}

/// Written before the send so a failed attempt is not retried on every run.
fn mark_sent(env: &BTreeMap<String, String>, today: u64) {
    let dir = state_dir(env);
    let _ = crate::config::ensure_private_dir(&dir);
    let _ = crate::config::write_private(&dir.join(STAMP_FILE), format!("{today}\n").as_bytes());
}

/// `suppress_send` is true for dev builds and background update children:
/// they never send, but `ANYR_TELEMETRY_DEBUG=1` still shows the payload.
fn decide(
    command: &str,
    env: &BTreeMap<String, String>,
    today: u64,
    suppress_send: bool,
) -> Action {
    if !enabled(env) {
        return Action::Skip;
    }
    let debug = env_flag(env, "ANYR_TELEMETRY_DEBUG") == Some(true);
    if !debug && (suppress_send || sent_today(env, today)) {
        return Action::Skip;
    }
    let (id, first_run) = load_or_create_id(env);
    if first_run {
        eprintln!("{}", notice());
    }
    let body = payload(id.as_deref(), command, in_ci(env)).to_string();
    if debug {
        return Action::Debug(body);
    }
    mark_sent(env, today);
    Action::Send(body)
}

/// Daily ping on startup. Never blocks and never prints errors.
pub fn on_startup(command: &str, env: &BTreeMap<String, String>) {
    let suppress = running_from_cargo_target() || env.contains_key("ANYR_AUTO_CHILD");
    match decide(command, env, today(), suppress) {
        Action::Skip => {}
        Action::Debug(body) => eprintln!("anyr telemetry (debug, not sent): {body}"),
        Action::Send(body) => spawn_detached(&body, env),
    }
}

/// Fire-and-forget `anyr __telemetry <json>`; the child outlives this process.
fn spawn_detached(body: &str, env: &BTreeMap<String, String>) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("__telemetry")
        .arg(body)
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

/// Body of the detached child: one POST, one attempt, response ignored.
pub fn send_ping(args: &[String], env: &BTreeMap<String, String>) {
    let (Some(body), Some(url)) = (args.first(), endpoint(env)) else {
        return;
    };
    let _ = crate::http::http_post_json_timeout(&url, body, TIMEOUT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::time::Instant;

    fn temp_home() -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "anyr-telemetry-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&home).unwrap();
        home
    }

    /// Env with a fresh home and no opt-out. CI is absent on purpose, so the
    /// tests behave the same on a CI runner.
    fn env_in(home: &std::path::Path) -> BTreeMap<String, String> {
        BTreeMap::from([(
            "ANYROUTER_HOME".to_string(),
            home.to_string_lossy().into_owned(),
        )])
    }

    fn with(env: &BTreeMap<String, String>, k: &str, v: &str) -> BTreeMap<String, String> {
        let mut env = env.clone();
        env.insert(k.into(), v.into());
        env
    }

    /// An opted-out run sends nothing and writes nothing (no id file either).
    fn assert_silent(env: &BTreeMap<String, String>, home: &std::path::Path) {
        assert_eq!(decide("claude", env, 100, false), Action::Skip);
        assert!(!home.join(ID_FILE).exists());
        assert!(!home.join(STAMP_FILE).exists());
    }

    #[test]
    fn sends_by_default() {
        let home = temp_home();
        assert!(matches!(
            decide("claude", &env_in(&home), 100, false),
            Action::Send(_)
        ));
    }

    #[test]
    fn anyr_telemetry_disabled_sends_nothing() {
        let home = temp_home();
        assert_silent(&with(&env_in(&home), "ANYR_TELEMETRY_DISABLED", "1"), &home);
    }

    #[test]
    fn do_not_track_sends_nothing() {
        let home = temp_home();
        assert_silent(&with(&env_in(&home), "DO_NOT_TRACK", "1"), &home);
    }

    #[test]
    fn ci_sends_nothing() {
        for value in ["1", "true", "yes"] {
            let home = temp_home();
            assert_silent(&with(&env_in(&home), "CI", value), &home);
        }
    }

    #[test]
    fn falsy_ci_values_do_not_count_as_ci() {
        for value in ["", "0", "false"] {
            let home = temp_home();
            let env = with(&env_in(&home), "CI", value);
            assert!(matches!(
                decide("claude", &env, 100, false),
                Action::Send(_)
            ));
        }
    }

    #[test]
    fn config_telemetry_false_sends_nothing() {
        let home = temp_home();
        fs::write(home.join("config.yaml"), "telemetry: false\n").unwrap();
        assert_silent(&env_in(&home), &home);
    }

    #[test]
    fn config_telemetry_true_keeps_it_on() {
        let home = temp_home();
        fs::write(home.join("config.yaml"), "telemetry: true\n").unwrap();
        assert!(matches!(
            decide("claude", &env_in(&home), 100, false),
            Action::Send(_)
        ));
    }

    #[test]
    fn empty_endpoint_override_disables() {
        let home = temp_home();
        assert_silent(
            &with(&env_in(&home), "ANYR_TELEMETRY_ENDPOINT", "  "),
            &home,
        );
    }

    #[test]
    fn endpoint_defaults_and_overrides() {
        let env = BTreeMap::new();
        assert_eq!(endpoint(&env).as_deref(), Some(ENDPOINT));
        let env = with(&env, "ANYR_TELEMETRY_ENDPOINT", "http://127.0.0.1:9/x");
        assert_eq!(endpoint(&env).as_deref(), Some("http://127.0.0.1:9/x"));
    }

    #[test]
    fn at_most_one_ping_per_day() {
        let home = temp_home();
        let env = env_in(&home);
        assert!(matches!(
            decide("claude", &env, 100, false),
            Action::Send(_)
        ));
        assert_eq!(decide("models", &env, 100, false), Action::Skip);
        assert!(matches!(
            decide("claude", &env, 101, false),
            Action::Send(_)
        ));
    }

    #[test]
    fn suppressed_runs_do_not_spend_the_daily_ping() {
        let home = temp_home();
        let env = env_in(&home);
        assert_eq!(decide("claude", &env, 100, true), Action::Skip);
        assert!(!home.join(STAMP_FILE).exists());
        assert!(matches!(
            decide("claude", &env, 100, false),
            Action::Send(_)
        ));
    }

    #[test]
    fn debug_prints_every_run_and_never_sends_or_stamps() {
        let home = temp_home();
        let env = with(&env_in(&home), "ANYR_TELEMETRY_DEBUG", "1");
        assert!(matches!(
            decide("claude", &env, 100, false),
            Action::Debug(_)
        ));
        assert!(matches!(
            decide("claude", &env, 100, false),
            Action::Debug(_)
        ));
        assert!(matches!(
            decide("claude", &env, 100, true),
            Action::Debug(_)
        ));
        assert!(!home.join(STAMP_FILE).exists());
    }

    #[test]
    fn debug_still_respects_opt_out() {
        let home = temp_home();
        let env = with(&env_in(&home), "ANYR_TELEMETRY_DEBUG", "1");
        assert_silent(&with(&env, "DO_NOT_TRACK", "1"), &home);
    }

    #[test]
    fn payload_has_exactly_the_documented_fields() {
        let v = payload(
            Some("3f2c9a7e-1d4b-4c8e-9a52-7b1e0d6f4a21"),
            "claude",
            false,
        );
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["arch", "ci", "event", "id", "os", "props", "version"]
        );
        assert_eq!(v["event"], "ping");
        assert_eq!(v["version"], VERSION);
        assert_eq!(v["ci"], false);
        assert_eq!(v["props"], json!({ "command": "claude" }));
        assert!(v["os"].as_str().unwrap().len() <= 16);
        assert!(v["arch"].as_str().unwrap().len() <= 16);
    }

    #[test]
    fn payload_without_a_saved_id_omits_it() {
        assert!(payload(None, "claude", false).get("id").is_none());
    }

    #[test]
    fn command_label_never_carries_user_input() {
        assert_eq!(command_label("cc"), "claude");
        assert_eq!(command_label("update"), "upgrade");
        assert_eq!(command_label("--help"), "help");
        assert_eq!(command_label("menu"), "menu");
        for typed in ["sk-ar-v1-secret", "/home/me/project", "rm -rf", ""] {
            assert_eq!(command_label(typed), "unknown");
        }
    }

    #[test]
    fn payload_has_no_pii() {
        let home = temp_home();
        let env = with(&env_in(&home), "HOME", "/home/someone");
        let Action::Send(body) = decide("/home/someone/secret-project", &env, 100, false) else {
            panic!("expected a send");
        };
        assert!(!body.contains(home.to_str().unwrap()));
        assert!(!body.contains("someone"));
        assert!(!body.contains("secret-project"));
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["props"], json!({ "command": "unknown" }));
    }

    #[test]
    fn id_is_a_random_uuid_saved_once() {
        let home = temp_home();
        let env = env_in(&home);
        let (first, fresh) = load_or_create_id(&env);
        let first = first.unwrap();
        assert!(fresh);
        let v4 = |s: &str| {
            let p: Vec<_> = s.split('-').map(str::len).collect();
            p == [8, 4, 4, 4, 12] && s.as_bytes()[14] == b'4' && "89ab".contains(&s[19..20])
        };
        assert!(v4(&first), "{first}");
        let (again, fresh) = load_or_create_id(&env);
        assert_eq!(again.as_deref(), Some(first.as_str()));
        assert!(!fresh);
        assert_ne!(random_uuid(), random_uuid());
        assert_eq!(
            fs::read_to_string(home.join(ID_FILE)).unwrap().trim(),
            first
        );
    }

    #[test]
    fn corrupt_id_file_is_replaced() {
        let home = temp_home();
        fs::write(home.join(ID_FILE), "not an id\n").unwrap();
        let (id, fresh) = load_or_create_id(&env_in(&home));
        assert!(fresh && is_valid_id(&id.unwrap()));
    }

    #[test]
    fn unwritable_home_still_pings_without_an_id() {
        let home = temp_home();
        let blocker = home.join("file");
        fs::write(&blocker, "x").unwrap();
        let env = env_in(&blocker.join("sub"));
        let Action::Send(body) = decide("claude", &env, 100, false) else {
            panic!("expected a send");
        };
        assert!(serde_json::from_str::<Value>(&body)
            .unwrap()
            .get("id")
            .is_none());
    }

    /// One-shot local server; returns its URL and the raw request it saw.
    fn serve_once(reply: bool) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/anyrouter-cli", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_millis(300))).ok();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = sock.read(&mut buf) {
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
            }
            if reply {
                let _ = sock.write_all(b"HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\n\r\n");
            } else {
                std::thread::sleep(Duration::from_secs(3));
            }
            String::from_utf8_lossy(&seen).into_owned()
        });
        (url, handle)
    }

    #[test]
    fn send_ping_posts_the_json_body() {
        let (url, server) = serve_once(true);
        let env = with(&BTreeMap::new(), "ANYR_TELEMETRY_ENDPOINT", &url);
        let body = payload(Some("id"), "claude", false).to_string();
        send_ping(std::slice::from_ref(&body), &env);
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /anyrouter-cli "), "{request}");
        assert!(request.to_ascii_lowercase().contains("application/json"));
        assert!(request.ends_with(&body), "{request}");
    }

    #[test]
    fn send_ping_gives_up_within_the_timeout() {
        let (url, _server) = serve_once(false);
        let env = with(&BTreeMap::new(), "ANYR_TELEMETRY_ENDPOINT", &url);
        let start = Instant::now();
        send_ping(&["{}".to_string()], &env);
        assert!(start.elapsed() < Duration::from_millis(2500));
    }

    #[test]
    fn send_ping_with_disabled_endpoint_returns_quietly() {
        let env = with(&BTreeMap::new(), "ANYR_TELEMETRY_ENDPOINT", "");
        send_ping(&["{}".to_string()], &env);
        send_ping(&[], &env);
    }
}
