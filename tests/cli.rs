use std::process::Command;

fn anyr() -> Command {
    Command::new(env!("CARGO_BIN_EXE_anyr"))
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = anyr().args(args).output().expect("spawn anyr");
    (
        out.status.code().unwrap_or(1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn help_lists_login_claude_account_and_spawn_targets() {
    let (code, stdout, stderr) = run(&["--help"]);
    assert_eq!(code, 0, "stderr={stderr}");
    for word in ["login", "claude", "account", "usage", "whoami"] {
        assert!(stdout.contains(word), "missing {word} in:\n{stdout}");
    }
    for target in ["claude", "cc", "codex", "grok", "opencode", "pi", "pool", "poolside"] {
        assert!(
            stdout.lines().any(|l| l.trim_start().starts_with(target)),
            "missing spawn target {target} in:\n{stdout}"
        );
    }
    assert!(
        stdout.contains("github.com/anyrouter-dev/cli")
            || stdout.contains("raw.githubusercontent.com/anyrouter-dev/cli"),
        "help must document the public GitHub install, got:\n{stdout}"
    );
    assert!(stdout.contains("setup.sh"), "{stdout}");
}

#[test]
fn version_matches_0_1() {
    let (code, stdout, stderr) = run(&["--version"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let ver = stdout.trim();
    assert!(
        ver.starts_with("0.1."),
        "expected --version to match ^0\\.1\\. got {ver:?}"
    );
}

#[test]
fn login_usage_whoami_help_exit_zero() {
    for cmd in ["login", "usage", "whoami"] {
        let (code, stdout, stderr) = run(&[cmd, "--help"]);
        assert_eq!(code, 0, "{cmd} --help failed: {stdout}{stderr}");
    }
}

#[test]
fn documented_commands_are_known_not_unknown() {
    // Drive the shipped binary — not packages/cli/bin/cli.mjs.
    let commands = [
        "chat",
        "setup",
        "models",
        "config",
        "skills",
        "relay",
        "byok",
        "task",
        "delegate",
        "keys",
        "logs",
        "audit",
        "logout",
        "transactions",
        "account",
        "login",
        "usage",
        "whoami",
        "claude",
        "codex",
        "grok",
        "opencode",
        "pi",
        "pool",
        "poolside",
        "upgrade",
        "update",
    ];
    for cmd in commands {
        let (code, stdout, stderr) = run(&[cmd, "--help"]);
        let combined = format!("{stdout}{stderr}");
        assert_eq!(code, 0, "{cmd} --help failed: {combined}");
        assert!(
            !combined.contains("Unknown command"),
            "{cmd} treated as unknown:\n{combined}"
        );
        assert!(!stdout.is_empty(), "{cmd} --help printed nothing");
    }
}

#[test]
fn spawn_targets_dry_run_inject_gateway_and_redact_key() {
    let key = "sk-ar-v1-testkey";
    let cases: &[(&[&str], &str)] = &[
        (&["claude", "--dry-run", "--yes", "--key", key], "ANTHROPIC_BASE_URL"),
        (&["codex", "--dry-run", "--yes", "--key", key], "OPENAI_BASE_URL"),
        (
            &["opencode", "--dry-run", "--yes", "--key", key],
            "OPENCODE_CONFIG_CONTENT",
        ),
        (&["pi", "--dry-run", "--yes", "--key", key], "ANYROUTER_API_KEY"),
    ];
    for (args, marker) in cases {
        let out = anyr()
            .args(*args)
            .env_remove("ANYROUTER_API_KEY")
            .output()
            .expect("dry-run");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code().unwrap_or(1),
            0,
            "{args:?} stderr={stderr} stdout={stdout}"
        );
        assert!(
            stdout.contains(marker),
            "{args:?} missing {marker} in:\n{stdout}"
        );
        assert!(
            !stdout.contains(key),
            "{args:?} leaked full key:\n{stdout}"
        );
    }
}

#[test]
fn pi_dry_run_uses_anyrouter_provider() {
    let key = "sk-ar-v1-testkey";
    let (code, stdout, stderr) = {
        let out = anyr()
            .args([
                "pi",
                "--dry-run",
                "--yes",
                "--key",
                key,
                "--model",
                "z-ai/glm-4.7-flash",
            ])
            .env_remove("ANYROUTER_API_KEY")
            .output()
            .expect("pi dry-run");
        (
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("command: pi"), "{stdout}");
    assert!(stdout.contains("--provider"), "{stdout}");
    assert!(stdout.contains("anyrouter"), "{stdout}");
    assert!(stdout.contains("--model"), "{stdout}");
    assert!(stdout.contains("z-ai/glm-4.7-flash"), "{stdout}");
    assert!(stdout.contains("PI_MODELS_JSON"), "{stdout}");
    assert!(stdout.contains("anyrouter.dev/api/v1"), "{stdout}");
    assert!(!stdout.contains(key), "full key leaked:\n{stdout}");
}

#[test]
fn claude_dry_run_with_key_prints_base_and_redacts_secret() {
    let key = "sk-ar-v1-testkey";
    let (code, stdout, stderr) = {
        let out = anyr()
            .args(["claude", "--dry-run", "--yes", "--key", key])
            .env_remove("ANYROUTER_API_KEY")
            .output()
            .expect("dry-run");
        (
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("ANTHROPIC_BASE_URL"), "{stdout}");
    assert!(!stdout.contains(key), "full key leaked:\n{stdout}");
}

#[test]
fn model_without_value_errors() {
    let (code, stdout, stderr) = run(&["claude", "--model"]);
    assert_ne!(code, 0);
    assert!(format!("{stdout}{stderr}").contains("requires a value"));
}

#[test]
fn upgrade_help_mentions_channel_stable_beta() {
    let (code, stdout, stderr) = run(&["upgrade", "--help"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let combined = format!("{stdout}{stderr}");
    assert!(
        !combined.contains("Unknown command"),
        "upgrade treated as unknown:\n{combined}"
    );
    assert!(stdout.contains("channel"), "{stdout}");
    assert!(stdout.contains("stable"), "{stdout}");
    assert!(stdout.contains("beta"), "{stdout}");
}

#[test]
fn update_is_upgrade_alias() {
    let (code, stdout, stderr) = run(&["update", "--help"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let combined = format!("{stdout}{stderr}");
    assert!(
        !combined.contains("Unknown command"),
        "update treated as unknown:\n{combined}"
    );
}

#[test]
fn upgrade_check_flag_is_known() {
    let fixture = std::env::temp_dir().join("anyr-cli-upgrade-check.json");
    std::fs::write(
        &fixture,
        r#"[{"tag_name":"v0.1.0","prerelease":false,"draft":false,"assets":[{"name":"anyr-linux-x86_64"}]}]"#,
    )
    .expect("write fixture");
    let out = anyr()
        .args(["upgrade", "--check"])
        .env("ANYR_RELEASES_JSON", &fixture)
        .output()
        .expect("upgrade --check");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");
    assert_eq!(out.status.code().unwrap_or(1), 0, "{combined}");
    assert!(
        !combined.contains("Unknown command"),
        "upgrade --check treated as unknown:\n{combined}"
    );
    assert!(stdout.contains("up to date") || stdout.contains("Already up to date"), "{stdout}");
}

fn fixture_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/releases.json")
}

#[test]
fn upgrade_check_newer_stable_would_upgrade() {
    let (code, stdout, stderr) = {
        let out = anyr()
            .args(["upgrade", "--check", "--channel", "stable"])
            .env("ANYR_RELEASES_JSON", fixture_path())
            .env_remove("ANYR_CHANNEL")
            .output()
            .expect("upgrade --check stable");
        (
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("channel: stable"), "{stdout}");
    assert!(stdout.contains("latest: 0.1.1"), "{stdout}");
    assert!(stdout.contains("update available"), "{stdout}");
    assert!(stdout.contains("github.com/anyrouter-dev/cli/releases/download/"), "{stdout}");
}

#[test]
fn upgrade_check_beta_selects_prerelease() {
    let (code, stdout, stderr) = {
        let out = anyr()
            .args(["upgrade", "--check", "--channel", "beta"])
            .env("ANYR_RELEASES_JSON", fixture_path())
            .env("ANYR_CHANNEL", "stable")
            .output()
            .expect("upgrade --check beta");
        (
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("channel: beta"), "{stdout}");
    assert!(stdout.contains("latest: 0.1.2-beta.1"), "{stdout}");
    assert!(stdout.contains("update available"), "{stdout}");
}

#[test]
fn upgrade_does_not_print_full_sk_ar_key() {
    let key = "sk-ar-v1-secret-value";
    let (code, stdout, stderr) = {
        let out = anyr()
            .args(["upgrade", "--check", "--dry-run"])
            .env("ANYR_RELEASES_JSON", fixture_path())
            .env("ANYROUTER_API_KEY", key)
            .output()
            .expect("upgrade dry-run");
        (
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    assert_eq!(code, 0, "stderr={stderr}");
    let combined = format!("{stdout}{stderr}");
    assert!(!combined.contains(key), "full key leaked:\n{combined}");
}
