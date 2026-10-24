//! `anyr config get|set|unset launcher.*` and what a launch does with it.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const KEY: &str = "sk-ar-v1-settingstest0123";

fn home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("anyr-settings-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn anyr(home: &PathBuf, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_anyr"))
        .env("ANYROUTER_HOME", home)
        .env("ANYR_NO_UPDATE", "1")
        .env("ANYR_NO_CATALOG", "1")
        .env("NO_COLOR", "1")
        .env_remove("ANYROUTER_API_KEY")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn set_get_unset_round_trip_with_scriptable_exit_codes() {
    let h = home("roundtrip");
    let unset = anyr(&h, &["config", "get", "launcher.codex.model"]);
    assert_eq!(
        (unset.status.code(), stdout(&unset)),
        (Some(1), String::new())
    );
    let set = anyr(
        &h,
        &["config", "set", "launcher.codex.model", "openai/gpt-5.5"],
    );
    assert_eq!(set.status.code(), Some(0));
    let got = anyr(&h, &["config", "get", "launcher.codex.model", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&got)).unwrap();
    assert_eq!(v["value"], "openai/gpt-5.5");
    assert_eq!(
        anyr(&h, &["config", "unset", "launcher.codex.model"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        anyr(&h, &["config", "get", "launcher.codex.model"])
            .status
            .code(),
        Some(1)
    );
    let bad = anyr(&h, &["config", "get", "launcher.vim.model"]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("agents: claude"));
}

#[test]
fn per_agent_args_and_model_shape_the_launch() {
    let h = home("launch");
    anyr(
        &h,
        &[
            "config",
            "set",
            "launcher.claude.model",
            "minimax/minimax-m3.1-flash",
        ],
    );
    anyr(
        &h,
        &[
            "config",
            "set",
            "launcher.claude.args",
            "--",
            "--verbose --add-dir /srv",
        ],
    );
    let dry = anyr(&h, &["claude", "--dry-run", "--key", KEY, "--", "--resume"]);
    let out = stdout(&dry);
    assert_eq!(dry.status.code(), Some(0), "{out}");
    assert!(
        out.contains(r#"args: ["--verbose","--add-dir","/srv","--resume"]"#),
        "saved args first, the user's own last:\n{out}"
    );
    assert!(
        out.contains("ANTHROPIC_MODEL=minimax/minimax-m3.1-flash"),
        "{out}"
    );
    let view = anyr(&h, &["config", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&view)).unwrap();
    let claude = &v["launcher"]["agents"]["claude"];
    assert_eq!(claude["model_source"], "agent");
    assert_eq!(
        claude["args"],
        serde_json::json!(["--verbose", "--add-dir", "/srv"])
    );
    assert!(
        v.get("claude_haiku").is_none(),
        "tiers live under launcher.claude"
    );
}
