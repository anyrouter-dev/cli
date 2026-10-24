//! `anyr qwen|goose|aider|crush|kilo`: each agent reaches AnyRouter with the
//! key from the environment only, on the model anyr picked, and the user's own
//! config files for that agent are never written.

mod common;
use common::{serve, KEY};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const MODEL: &str = "z-ai/glm-4.7-flash";

/// Reads the endpoint, key and model the way each real agent does (from the
/// variables and files anyr hands it), then POSTs one chat completion. Fails
/// when the key shows up in argv.
const FAKE_AGENT: &str = r#"#!/usr/bin/env python3
import json, os, sys, urllib.request, urllib.error
agent = os.path.basename(sys.argv[0]).removeprefix("fake-")
env = os.environ
if sys.argv[1:] == ["--version"]:
    print("0.0.14")
    raise SystemExit(0)
if any(env.get("ANYR_TEST_KEY", "-") in a for a in sys.argv):
    raise SystemExit("key in argv")
def arg(flag):
    return sys.argv[sys.argv.index(flag) + 1] if flag in sys.argv else None
if agent == "qwen":
    url, key, model = env["OPENAI_BASE_URL"] + "/chat/completions", env["OPENAI_API_KEY"], arg("-m")
elif agent == "goose":
    url = env["OPENAI_HOST"] + "/" + env["OPENAI_BASE_PATH"]
    key, model = env["OPENAI_API_KEY"], env["GOOSE_MODEL"]
    assert env["GOOSE_PROVIDER"] == "openai"
elif agent == "aider":
    url, key = env["AIDER_OPENAI_API_BASE"] + "/chat/completions", env["AIDER_OPENAI_API_KEY"]
    model = env["AIDER_MODEL"].removeprefix("openai/")
elif agent == "crush":
    cfg = json.load(open(os.path.join(env["CRUSH_GLOBAL_CONFIG"], "crush.json")))
    p = cfg["providers"]["anyrouter"]
    url, key = p["base_url"] + "/chat/completions", env[p["api_key"].removeprefix("$")]
    model = cfg["models"]["large"]["model"]
    assert env["CRUSH_GLOBAL_DATA"] != os.path.expanduser("~/.local/share/crush")
elif agent == "fx":
    cfg = json.load(open(os.path.join(env["HOME"], ".fx", "settings.json")))
    c = cfg["providers"][env["FX_PROVIDER"]]
    url, key, model = c["base_url"] + "/chat/completions", env[c["auth"]["env"]], env["FX_MODEL"]
elif agent == "kilo":
    cfg = json.loads(env["KILO_CONFIG_CONTENT"])
    o = cfg["provider"]["anyrouter"]["options"]
    url, key = o["baseURL"] + "/chat/completions", env[o["apiKey"][len("{env:"):-1]]
    model = arg("-m").removeprefix("anyrouter/")
body = json.dumps({"model": model}).encode()
req = urllib.request.Request(url, data=body, headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"})
try:
    urllib.request.urlopen(req, timeout=5)
except urllib.error.HTTPError:
    pass
"#;

/// The user's own config for each agent, planted before the launch.
const USER_FILES: &[(&str, &str, &str)] = &[
    (
        "qwen",
        ".qwen/settings.json",
        r#"{"model":{"name":"qwen3-coder-plus"}}"#,
    ),
    (
        "goose",
        ".config/goose/config.yaml",
        "GOOSE_PROVIDER: anthropic\n",
    ),
    ("aider", ".aider.conf.yml", "model: sonnet\n"),
    (
        "crush",
        ".config/crush/crush.json",
        r#"{"options":{"debug":true},"providers":{"mine":{"type":"openai-compat","base_url":"https://example.invalid/v1"}}}"#,
    ),
    ("crush", ".config/crush/crushrc", "# the user's crushrc\n"),
    ("kilo", ".config/kilo/kilo.json", r#"{"theme":"mine"}"#),
];

fn temp_dir(tag: &str) -> PathBuf {
    common::unique_temp_dir(&format!("anyr-more-{tag}"))
}

fn plant_user_files(home: &Path, agent: &str) {
    for (a, rel, body) in USER_FILES.iter().filter(|(a, ..)| *a == agent) {
        let path = home.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap_or_else(|e| panic!("{a}: {e}"));
    }
}

fn anyr(dir: &Path, home: &Path, agent: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_anyr"));
    cmd.env("ANYR_NO_UPDATE", "1")
        .env("ANYR_NO_CATALOG", "1")
        .env("NO_COLOR", "1")
        .env("HOME", home)
        .env("ANYROUTER_HOME", dir.join("anyr"))
        .env("ANYROUTER_API_KEY", KEY)
        .env("ANYR_TEST_KEY", KEY)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CRUSH_GLOBAL_CONFIG")
        .args([agent, "--model", MODEL, "--yes", "--no-mcp"])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    cmd
}

#[cfg(unix)]
#[test]
fn each_new_agent_sends_the_anyrouter_key_from_env_and_leaves_user_config_alone() {
    use std::os::unix::fs::PermissionsExt;
    for agent in ["qwen", "goose", "aider", "crush", "kilo"] {
        let dir = temp_dir(agent);
        let home = dir.join("home");
        plant_user_files(&home, agent);
        let fake = dir.join(format!("fake-{agent}"));
        std::fs::write(&fake, FAKE_AGENT).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (base, rx) = serve(vec![(401, r#"{"error":{"message":"x"}}"#.into())]);
        let out = anyr(&dir, &home, agent, &["--base-url", &base, "--command-path"])
            .arg(&fake)
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{agent}: {err}");
        let seen = rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| panic!("{agent} sent nothing: {err}"));
        assert_eq!(seen.path, "/api/v1/chat/completions", "{agent}");
        assert_eq!(seen.auth, format!("Bearer {KEY}"), "{agent}");
        assert!(seen.body.contains(MODEL), "{agent}: {}", seen.body);
        for (_, rel, body) in USER_FILES.iter().filter(|(a, ..)| *a == agent) {
            assert_eq!(
                std::fs::read_to_string(home.join(rel)).unwrap(),
                *body,
                "{agent} launch changed the user's {rel}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn crush_private_config_keeps_the_users_setup_and_holds_no_key() {
    let dir = temp_dir("crush-private");
    let home = dir.join("home");
    plant_user_files(&home, "crush");
    let out = anyr(
        &dir,
        &home,
        "crush",
        &["--dry-run", "--command-path", "/nonexistent/crush"],
    )
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let private = dir.join("anyr").join("crush");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("CRUSH_GLOBAL_CONFIG={}", private.display())),
        "{stdout}"
    );
    let raw = std::fs::read_to_string(private.join("crush.json")).unwrap();
    assert!(
        !raw.contains(KEY),
        "crush.json must not hold the key:\n{raw}"
    );
    let cfg: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        cfg["providers"]["anyrouter"]["api_key"],
        "$ANYROUTER_API_KEY"
    );
    assert_eq!(cfg["models"]["large"]["model"], MODEL);
    assert_eq!(cfg["options"]["debug"], true, "user option dropped");
    assert!(
        cfg["providers"]["mine"].is_object(),
        "user provider dropped"
    );
    assert_eq!(
        std::fs::read_to_string(private.join("crushrc")).unwrap(),
        "# the user's crushrc\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `anyr <agent> --dry-run` env lines as a map.
fn dry_env(
    agent: &str,
    extra: &[&str],
) -> (Vec<String>, std::collections::BTreeMap<String, String>) {
    let dir = temp_dir(&format!("dry-{agent}"));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let out = anyr(
        &dir,
        &home,
        agent,
        &[
            "--dry-run",
            "--base-url",
            "https://gw.example/api",
            "--command-path",
            "/nonexistent/agent",
        ],
    )
    .args(extra)
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        !stdout.contains(KEY),
        "{agent} dry run printed the key:\n{stdout}"
    );
    let args = stdout
        .lines()
        .find_map(|l| l.strip_prefix("args: "))
        .map(|j| serde_json::from_str(j).unwrap())
        .unwrap();
    let env = stdout
        .lines()
        .skip_while(|l| *l != "env:")
        .skip(1)
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (args, env)
}

#[test]
fn dry_run_points_each_agent_at_anyrouter_with_the_chosen_model() {
    let (args, env) = dry_env("qwen", &[]);
    assert_eq!(env["OPENAI_BASE_URL"], "https://gw.example/api/v1");
    assert_eq!(env["OPENAI_MODEL"], MODEL);
    assert_eq!(
        args,
        ["-m", MODEL],
        "settings.json model.name beats OPENAI_MODEL"
    );

    let (_, env) = dry_env("goose", &[]);
    assert_eq!(env["OPENAI_HOST"], "https://gw.example");
    assert_eq!(env["OPENAI_BASE_PATH"], "api/v1/chat/completions");
    assert_eq!(env["GOOSE_PROVIDER"], "openai");
    assert_eq!(env["GOOSE_MODEL"], MODEL);

    let (_, env) = dry_env("aider", &[]);
    assert_eq!(env["AIDER_OPENAI_API_BASE"], "https://gw.example/api/v1");
    assert_eq!(env["AIDER_MODEL"], format!("openai/{MODEL}"));

    let (args, env) = dry_env("kilo", &[]);
    let cfg: serde_json::Value = serde_json::from_str(&env["KILO_CONFIG_CONTENT"]).unwrap();
    assert_eq!(cfg["model"], format!("anyrouter/{MODEL}"));
    assert_eq!(args, ["-m", &format!("anyrouter/{MODEL}")]);
    let options = &cfg["provider"]["anyrouter"]["options"];
    assert_eq!(options["baseURL"], "https://gw.example/api/v1");
    assert_eq!(options["apiKey"], "{env:ANYROUTER_API_KEY}");
}

#[test]
fn goose_yolo_sets_its_mode_instead_of_a_flag() {
    let (args, env) = dry_env("goose", &["--yolo", "run", "-t", "hi"]);
    assert_eq!(env["GOOSE_MODE"], "auto");
    assert_eq!(args, ["run", "-t", "hi"]);
    let (_, env) = dry_env("goose", &["run", "-t", "hi"]);
    assert!(!env.contains_key("GOOSE_MODE"), "no --yolo, no auto mode");
}

#[test]
fn missing_binary_names_the_install_command() {
    for (agent, install) in [
        ("qwen", "@qwen-code/qwen-code"),
        ("goose", "download_cli.sh"),
        ("aider", "aider.chat/install.sh"),
        ("crush", "@charmland/crush"),
        ("kilo", "@kilocode/cli"),
        ("fx", "fx.sh/setup.sh"),
    ] {
        let dir = temp_dir(&format!("missing-{agent}"));
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let out = anyr(
            &dir,
            &home,
            agent,
            &["--command-path", "no-such-agent-binary"],
        )
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{agent}");
        assert!(err.contains(install), "{agent}: {err}");
        assert!(
            !home.join(".fx").exists(),
            "{agent}: a failed launch created ~/.fx"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn kilo_keeps_a_model_the_user_passed() {
    let (args, _) = dry_env("kilo", &["--", "-m", "other/model"]);
    assert_eq!(args, ["-m", "other/model"], "anyr must not add a second -m");
}

#[test]
fn crush_data_dir_is_private_and_the_users_data_file_is_untouched() {
    let dir = temp_dir("crush-data");
    let home = dir.join("home");
    plant_user_files(&home, "crush");
    let user_data = home.join(".local/share/crush/crush.json");
    std::fs::create_dir_all(user_data.parent().unwrap()).unwrap();
    let planted = r#"{"models":{"large":{"provider":"mine","model":"my-model"}}}"#;
    std::fs::write(&user_data, planted).unwrap();
    let out = anyr(
        &dir,
        &home,
        "crush",
        &["--dry-run", "--command-path", "/nonexistent/crush"],
    )
    .env("CRUSH_GLOBAL_DATA", home.join(".local/share/crush"))
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let private = dir.join("anyr").join("crush-data");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("CRUSH_GLOBAL_DATA={}", private.display())),
        "{stdout}"
    );
    assert!(private.is_dir());
    assert_eq!(std::fs::read_to_string(&user_data).unwrap(), planted);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn fx_gets_only_the_anyrouter_connection_in_its_settings() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("fx");
    let home = dir.join("home");
    let settings = home.join(".fx").join("settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(
        &settings,
        r#"{"provider":"gateway","theme":"dark","providers":{"local":{"protocol":"openai-chat-completions","base_url":"http://127.0.0.1:11434/v1","auth":{"type":"none"}}}}"#,
    )
    .unwrap();
    let before = std::fs::read_to_string(&settings).unwrap();
    let dry = anyr(
        &dir,
        &home,
        "fx",
        &["--dry-run", "--command-path", "/nonexistent/fx"],
    )
    .output()
    .unwrap();
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(String::from_utf8_lossy(&dry.stdout).contains("would set providers.anyrouter"));
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        before,
        "--dry-run wrote"
    );

    let fake = dir.join("fake-fx");
    std::fs::write(&fake, FAKE_AGENT).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let launch = |what: &str| {
        let (base, rx) = serve(vec![(401, r#"{"error":{"message":"x"}}"#.into())]);
        let out = anyr(&dir, &home, "fx", &["--base-url", &base, "--command-path"])
            .arg(&fake)
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{what}: {err}");
        let seen = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(seen.auth, format!("Bearer {KEY}"), "{what}");
        assert!(seen.body.contains(MODEL), "{what}: {}", seen.body);
        (base, std::fs::read_to_string(&settings).unwrap())
    };
    let (base, first) = launch("first launch");
    assert!(!first.contains(KEY), "settings.json must not hold the key");
    let cfg: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(
        cfg["provider"], "gateway",
        "the user's own provider choice changed"
    );
    assert_eq!(cfg["theme"], "dark");
    assert!(
        cfg["providers"]["local"].is_object(),
        "user connection dropped"
    );
    let conn = &cfg["providers"]["anyrouter"];
    assert_eq!(conn["base_url"], format!("{base}/v1"));
    assert_eq!(conn["auth"]["env"], "ANYROUTER_API_KEY");
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode(&settings),
        0o600,
        "fx ignores a settings.json others can read"
    );
    assert_eq!(mode(settings.parent().unwrap()), 0o700);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fx_connection_is_left_alone_when_already_current() {
    let conn = serde_json::json!({ "base_url": "https://gw.example/api/v1" });
    let once = anyr_cli::spawn::fx_settings_with(None, &conn).expect("first write");
    assert_eq!(anyr_cli::spawn::fx_settings_with(Some(once), &conn), None);
}

#[cfg(unix)]
#[test]
fn fx_settings_symlink_is_written_through_not_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("fx-link");
    let home = dir.join("home");
    let dotfiles = dir.join("dotfiles").join("fx-settings.json");
    std::fs::create_dir_all(dotfiles.parent().unwrap()).unwrap();
    std::fs::write(&dotfiles, r#"{"theme":"dark"}"#).unwrap();
    std::fs::create_dir_all(home.join(".fx")).unwrap();
    let link = home.join(".fx").join("settings.json");
    std::os::unix::fs::symlink(&dotfiles, &link).unwrap();
    let fake = dir.join("fake-fx");
    std::fs::write(&fake, FAKE_AGENT).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, _rx) = serve(vec![(401, r#"{"error":{"message":"x"}}"#.into())]);
    let out = anyr(&dir, &home, "fx", &["--base-url", &base, "--command-path"])
        .arg(&fake)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&dotfiles).unwrap()).unwrap();
    assert_eq!(cfg["theme"], "dark");
    assert!(cfg["providers"]["anyrouter"].is_object());
    let _ = std::fs::remove_dir_all(&dir);
}
