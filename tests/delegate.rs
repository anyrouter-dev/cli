//! `anyr delegate`: argv per target, redacted dry-run, one-hop guard, and a
//! fake agent on disk that proves env and exit code cross the process edge.

use std::path::PathBuf;
use std::process::{Command, Output};

const KEY: &str = "sk-ar-v1-fixture-key-0001";

fn anyr(home: &PathBuf) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_anyr"));
    c.env("ANYR_NO_UPDATE", "1")
        .env("NO_COLOR", "1")
        .env("ANYROUTER_HOME", home)
        .env_remove("ANYROUTER_API_KEY")
        .env_remove("ANYROUTER_DELEGATE_DEPTH");
    c
}

const GW: [&str; 4] = ["--base-url", "http://127.0.0.1:9/api", "--key", KEY];

fn home(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("anyr-delegate-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn text(o: &Output) -> (i32, String, String) {
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn dry_run_argv_per_target_and_key_redacted() {
    let cases = [
        ("claude", r#"["-p","fix the bug"]"#),
        ("codex", r#""exec","fix the bug"]"#),
        ("opencode", r#"["run","fix the bug"]"#),
    ];
    for (to, argv) in cases {
        let h = home(to);
        let out = anyr(&h)
            .args(["delegate", "--to", to, "fix the bug", "--dry-run", "--yes"])
            .args(GW)
            .output()
            .unwrap();
        let (code, stdout, stderr) = text(&out);
        assert_eq!(code, 0, "{to}: {stdout}{stderr}");
        assert!(stdout.contains(argv), "{to}: {stdout}");
        assert!(stdout.contains("ANYROUTER_DELEGATE_DEPTH=1"), "{stdout}");
        assert!(!stdout.contains(KEY), "{to} leaked the key: {stdout}");
    }
}

#[test]
fn model_flag_reaches_the_agent() {
    let h = home("model");
    let out = anyr(&h)
        .args([
            "delegate",
            "--to",
            "codex",
            "t",
            "--model",
            "z-ai/glm-4.7-flash",
            "--dry-run",
            "--yes",
        ])
        .args(GW)
        .output()
        .unwrap();
    let (code, stdout, _) = text(&out);
    assert_eq!(code, 0);
    assert!(stdout.contains("z-ai/glm-4.7-flash"), "{stdout}");
}

#[test]
fn unknown_target_and_missing_task_are_usage_errors() {
    let h = home("usage");
    let out = anyr(&h)
        .args(["delegate", "--to", "vim", "task", "--yes"])
        .args(GW)
        .output()
        .unwrap();
    let (code, _, stderr) = text(&out);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("unknown agent"), "{stderr}");
    let out = anyr(&h)
        .args(["delegate", "--to", "claude", "--yes"])
        .args(GW)
        .output()
        .unwrap();
    assert_eq!(text(&out).0, 2);
}

#[test]
fn second_hop_is_refused() {
    let h = home("hop");
    let out = anyr(&h)
        .env("ANYROUTER_DELEGATE_DEPTH", "1")
        .args(["delegate", "--to", "claude", "t", "--yes"])
        .args(GW)
        .output()
        .unwrap();
    let (code, _, stderr) = text(&out);
    assert_ne!(code, 0);
    assert!(stderr.contains("one hop"), "{stderr}");
}

#[test]
fn missing_agent_binary_is_a_clear_error() {
    let h = home("missing");
    let out = anyr(&h)
        .env("ANYROUTER_CODEX_PATH", "/nonexistent/anyr-fake-codex")
        .args(["delegate", "--to", "codex", "t", "--yes"])
        .args(GW)
        .output()
        .unwrap();
    let (code, _, stderr) = text(&out);
    assert_ne!(code, 0);
    assert!(
        stderr.contains("anyr-fake-codex") || stderr.to_lowercase().contains("install"),
        "{stderr}"
    );
}

#[test]
fn without_yes_and_without_terminal_it_refuses() {
    let h = home("noyes");
    let out = anyr(&h)
        .args(["delegate", "--to", "claude", "t"])
        .stdin(std::process::Stdio::null())
        .args(GW)
        .output()
        .unwrap();
    let (code, _, stderr) = text(&out);
    assert_ne!(code, 0);
    assert!(stderr.contains("--yes"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn fake_agent_gets_task_env_and_its_exit_code_propagates() {
    use std::os::unix::fs::PermissionsExt;
    let h = home("fake");
    let script = h.join("fake-claude.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\necho \"ARGS:$*\"\necho \"KEY:$ANTHROPIC_AUTH_TOKEN$ANTHROPIC_API_KEY\"\necho \"BASE:$ANTHROPIC_BASE_URL\"\necho \"DEPTH:$ANYROUTER_DELEGATE_DEPTH\"\nexit 7\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = anyr(&h)
        .env("ANYROUTER_CLAUDE_PATH", &script)
        .args(["delegate", "--to", "claude", "do it", "--yes"])
        .args(GW)
        .output()
        .unwrap();
    let (code, stdout, stderr) = text(&out);
    assert_eq!(code, 7, "{stdout}{stderr}");
    assert!(stdout.contains("ARGS:-p do it"), "{stdout}");
    assert!(stdout.contains(KEY), "{stdout}");
    assert!(stdout.contains("BASE:http://127.0.0.1:9"), "{stdout}");
    assert!(stdout.contains("DEPTH:1"), "{stdout}");
}
