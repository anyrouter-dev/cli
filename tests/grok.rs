//! `anyr grok` must send the AnyRouter key even when a Grok session is saved.

mod common;
use common::{serve, KEY};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Emulates Grok 1.0.52 credential precedence: a stored session in
/// `$GROK_HOME/auth.json` (default `$HOME/.grok`) beats everything. With no
/// session it runs the auth provider command and uses its `access_token`.
/// A non-empty `XAI_API_KEY` / `GROK_CODE_XAI_API_KEY` sends it through the
/// unreliable key probe, so the fake fails when either is set.
/// It then POSTs one chat completion to `$GROK_MODELS_BASE_URL`.
const FAKE_GROK: &str = r#"#!/usr/bin/env python3
import json, os, subprocess, sys, urllib.request, urllib.error
if sys.argv[1:3] != ["-m", "deepseek/deepseek-v4-flash"]:
    raise SystemExit("unknown model id (grok would open its own default)")
if os.environ.get("XAI_API_KEY") or os.environ.get("GROK_CODE_XAI_API_KEY"):
    raise SystemExit("Not signed in")
home = os.environ.get("GROK_HOME") or os.path.join(os.environ["HOME"], ".grok")
with open(os.path.join(home, "config.toml"), "a") as f:
    f.write("launch_wrote = true\n")
session = os.path.join(home, "auth.json")
if os.path.exists(session):
    token = json.load(open(session))["access_token"]
else:
    out = subprocess.run(os.environ["GROK_AUTH_PROVIDER_COMMAND"], shell=True, capture_output=True, text=True, check=True).stdout
    token = json.loads(out)["access_token"]
req = urllib.request.Request(os.environ["GROK_MODELS_BASE_URL"] + "/chat/completions", data=b"{}", headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
try:
    urllib.request.urlopen(req, timeout=5)
except urllib.error.HTTPError:
    pass
"#;

#[cfg(unix)]
#[test]
fn a_saved_grok_session_does_not_replace_the_anyrouter_key() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("anyr-grok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let user_home = dir.join("home");
    std::fs::create_dir_all(user_home.join(".grok")).unwrap();
    let session = user_home.join(".grok").join("auth.json");
    let session_json = r#"{"access_token":"grok-session-jwt"}"#;
    std::fs::write(&session, session_json).unwrap();
    std::fs::write(
        user_home.join(".grok").join("config.toml"),
        "[ui]\nyolo = false\n",
    )
    .unwrap();
    std::fs::create_dir_all(user_home.join(".grok").join("skills")).unwrap();
    let agent = dir.join("fake-grok");
    std::fs::write(&agent, FAKE_GROK).unwrap();
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();

    // An `anyr` from before this fix saved `auth_env: GROK_CODE_XAI_API_KEY`,
    // and a saved tool wins over the built-in one.
    std::fs::create_dir_all(dir.join("anyr")).unwrap();
    std::fs::write(
        dir.join("anyr").join("config.yaml"),
        "active_profile: default\nprofiles:\n  default:\n    api_key: sk-ar-v1-stale\ntools:\n  grok:\n    command: grok\n    base_url_env: GROK_MODELS_BASE_URL\n    auth_env: GROK_CODE_XAI_API_KEY\n    model_env: null\n    base_suffix: /v1\n    enable_gateway_model_discovery: false\n    shadow_env: null\n",
    )
    .unwrap();
    let (base, rx) = serve(vec![(401, r#"{"error":{"message":"x"}}"#.into())]);
    let out = Command::new(env!("CARGO_BIN_EXE_anyr"))
        .env("ANYR_NO_UPDATE", "1")
        .env("ANYR_NO_CATALOG", "1")
        .env("NO_COLOR", "1")
        .env("HOME", &user_home)
        .env("ANYROUTER_HOME", dir.join("anyr"))
        .env("ANYROUTER_API_KEY", KEY)
        .env_remove("GROK_HOME")
        .env("XAI_API_KEY", "xai-parent-shell-key")
        .args([
            "grok",
            "--model",
            "deepseek/deepseek-v4-flash",
            "--yes",
            "--no-mcp",
        ])
        .args(["--base-url", &base, "--command-path"])
        .arg(&agent)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let seen = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(seen.path, "/api/v1/chat/completions");
    assert_eq!(seen.auth, format!("Bearer {KEY}"));
    assert_eq!(std::fs::read_to_string(&session).unwrap(), session_json);
    // The user's own setup is shared into the AnyRouter home; the session is not.
    let grok_home = dir.join("anyr").join("grok");
    assert!(std::fs::read_to_string(grok_home.join("config.toml"))
        .unwrap()
        .starts_with("[ui]\nyolo = false\n"));
    assert_eq!(
        std::fs::read_to_string(user_home.join(".grok").join("config.toml")).unwrap(),
        "[ui]\nyolo = false\n",
        "a write to the AnyRouter copy must not reach the user's config"
    );
    assert!(!std::fs::symlink_metadata(grok_home.join("config.toml"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(grok_home.join("skills").is_dir());
    assert!(!grok_home.join("auth.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
