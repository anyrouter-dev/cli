use std::process::Command;

fn anyr() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_anyr"));
    cmd.env("ANYR_NO_UPDATE", "1");
    // Skip live catalog lookup so auto stays anyrouter/auto in tests.
    cmd.env("ANYR_NO_CATALOG", "1");
    cmd
}

/// Fresh empty ANYROUTER_HOME so launch tests assert on built-in defaults
/// instead of whatever happens to live in the developer's real config.
fn temp_home() -> std::path::PathBuf {
    let home = std::env::temp_dir().join(format!(
        "anyr-cli-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).expect("create temp home");
    home
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
    for word in ["auth", "claude", "keys", "usage", "models", "onboard"] {
        assert!(stdout.contains(word), "missing {word} in:\n{stdout}");
    }
    for target in [
        "claude", "cc", "codex", "grok", "opencode", "pi", "pool", "poolside",
    ] {
        assert!(
            stdout.lines().any(|l| l.trim_start().starts_with(target)),
            "missing spawn target {target} in:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("setup.sh") && !stdout.contains("Install:"),
        "help must not tell an already-installed binary how to install, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("anyrouter.dev/docs/cli"),
        "help must not end with a docs URL, got:\n{stdout}"
    );
    assert!(
        stdout.contains("anyr claude") || stdout.contains("anyr <command>"),
        "binary --help should name itself anyr, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("npx @anyr/cli"),
        "native anyr --help must not tell people to type npx, got:\n{stdout}"
    );
    for heading in ["CORE COMMANDS", "LAUNCH"] {
        assert!(
            stdout.contains(heading),
            "help should group commands under {heading}, got:\n{stdout}"
        );
    }
    assert!(
        stdout.contains("Sign in if needed") && stdout.contains("auth login"),
        "bare-command help should describe login-then-launcher, got:\n{stdout}"
    );
    assert!(
        stdout.contains("Open the interactive TUI") && stdout.contains("menu:"),
        "help should present the TUI as the default entry, got:\n{stdout}"
    );
    assert!(
        stdout.contains("▀█████████▄"),
        "help should render the official AR half-block mark, got:\n{stdout}"
    );
}

#[test]
fn no_args_non_tty_prints_grouped_help() {
    let (code, stdout, stderr) = run(&[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(
        stdout.contains("Sign in if needed") && stdout.contains("CORE COMMANDS"),
        "no-args should print grouped help when not a TTY, got:\n{stdout}"
    );
}

#[test]
fn help_follows_anyr_display_bin() {
    for (name, needle) in [
        ("ar", "ar claude"),
        ("anyrouter", "anyrouter claude"),
        ("npx @anyr/cli", "npx @anyr/cli claude"),
    ] {
        let out = anyr()
            .args(["--help"])
            .env("ANYR_DISPLAY_BIN", name)
            .output()
            .expect("spawn anyr");
        assert_eq!(out.status.code().unwrap_or(1), 0);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains(needle),
            "ANYR_DISPLAY_BIN={name} missing {needle:?} in:\n{stdout}"
        );
        assert!(
            stdout.contains(&format!("{name} <command>"))
                || stdout.contains(&format!("{name}                  Open the interactive TUI")),
            "ANYR_DISPLAY_BIN={name} missing usage line in:\n{stdout}"
        );
    }
}

#[test]
fn login_help_follows_display_bin() {
    let out = anyr()
        .args(["login", "--help"])
        .env("ANYR_DISPLAY_BIN", "ar")
        .output()
        .expect("spawn anyr");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code().unwrap_or(1), 0);
    assert!(stdout.contains("ar auth login"), "{stdout}");
    assert!(!stdout.contains("npx @anyr/cli"), "{stdout}");
}

#[test]
fn auth_help_lists_gh_style_subcommands() {
    let (code, stdout, stderr) = run(&["auth", "--help"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    for sub in ["login", "logout", "status", "token", "switch"] {
        assert!(stdout.contains(sub), "auth --help missing {sub}:\n{stdout}");
    }
}

#[test]
fn auth_login_help_is_nested() {
    let (code, stdout, stderr) = run(&["auth", "login", "--help"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("auth login"), "{stdout}");
}

#[test]
fn auth_unknown_subcommand_errors() {
    let (code, stdout, stderr) = run(&["auth", "nope"]);
    assert_ne!(code, 0);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("unknown command") && combined.contains("auth"),
        "{combined}"
    );
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
        "auth",
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
        "onboard",
        "impl",
        "plan",
        "fix",
        "deploy",
        "cp",
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
fn onboard_impl_prints_contract_prompt() {
    let (code, stdout, stderr) = run(&["onboard", "impl"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("ANYROUTER_API_KEY"), "{stdout}");
    assert!(stdout.contains("https://anyrouter.dev/api/v1"), "{stdout}");
    assert!(stdout.contains("https://anyrouter.dev/api"), "{stdout}");
}

#[test]
fn onboard_shortcuts_and_json() {
    for cmd in ["impl", "plan", "fix", "deploy"] {
        let (code, stdout, stderr) = run(&[cmd]);
        assert_eq!(code, 0, "{cmd} stderr={stderr}");
        assert!(!stdout.is_empty(), "{cmd} empty");
    }
    let (code, stdout, stderr) = run(&["onboard", "plan", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(
        stdout.contains("\"mode\":\"plan\"") || stdout.contains("\"mode\": \"plan\""),
        "{stdout}"
    );
    assert!(
        stdout.to_ascii_lowercase().contains("do not change"),
        "{stdout}"
    );
}

#[test]
fn onboard_without_mode_non_tty_errors() {
    let (code, stdout, stderr) = run(&["onboard"]);
    assert_ne!(code, 0);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("Specify a mode") || combined.contains("onboard --help"),
        "{combined}"
    );
}

#[test]
fn stub_help_is_honest() {
    let (code, stdout, stderr) = run(&["task", "--help"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("not yet in the native CLI"),
        "expected honest stub help, got:\n{stdout}"
    );
}

#[test]
fn spawn_targets_dry_run_inject_gateway_and_redact_key() {
    let key = "sk-ar-v1-testkey";
    let cases: &[(&[&str], &str)] = &[
        (
            &["claude", "--dry-run", "--yes", "--key", key],
            "ANTHROPIC_BASE_URL",
        ),
        (
            &["codex", "--dry-run", "--yes", "--key", key],
            "OPENAI_BASE_URL",
        ),
        (
            &["opencode", "--dry-run", "--yes", "--key", key],
            "OPENCODE_CONFIG_CONTENT",
        ),
        (
            &["pi", "--dry-run", "--yes", "--key", key],
            "ANYROUTER_API_KEY",
        ),
    ];
    for (args, marker) in cases {
        let out = anyr()
            .args(*args)
            .env("ANYROUTER_HOME", temp_home())
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
        assert!(!stdout.contains(key), "{args:?} leaked full key:\n{stdout}");
    }
}

#[test]
fn pi_dry_run_uses_anyrouter_provider() {
    let key = "sk-ar-v1-testkey";
    let dir = std::env::temp_dir().join(format!("anyr-pi-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.yaml");
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
                "--config",
                cfg.to_str().unwrap(),
            ])
            .env("ANYROUTER_HOME", temp_home())
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
    assert!(
        !stdout.contains("anyrouter/z-ai/glm-4.7-flash"),
        "Pi --model is the catalog id, not anyrouter/<id>:\n{stdout}"
    );
    assert!(stdout.contains("PI_CODING_AGENT_DIR"), "{stdout}");
    assert!(stdout.contains("ANYROUTER_API_KEY"), "{stdout}");
    assert!(stdout.contains("anyrouter.dev/api/v1"), "{stdout}");
    assert!(!stdout.contains("$ANYROUTER_API_KEY"), "{stdout}");
    assert!(!stdout.contains(key), "full key leaked:\n{stdout}");
    let models = std::fs::read_to_string(dir.join("pi").join("models.json")).unwrap();
    assert!(
        models.contains("\"apiKey\": \"ANYROUTER_API_KEY\""),
        "{models}"
    );
    assert!(models.contains("z-ai/glm-4.7-flash"), "{models}");
    let settings = std::fs::read_to_string(dir.join("pi").join("settings.json")).unwrap();
    assert!(
        settings.contains("\"defaultProvider\": \"anyrouter\""),
        "{settings}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn claude_dry_run_with_key_prints_base_and_redacts_secret() {
    let key = "sk-ar-v1-testkey";
    let (code, stdout, stderr) = {
        let out = anyr()
            .args([
                "claude",
                "--dry-run",
                "--yes",
                "--key",
                key,
                "--model",
                "auto",
            ])
            .env("ANYROUTER_HOME", temp_home())
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
    assert!(
        stdout.contains("ANTHROPIC_MODEL=anyrouter/auto"),
        "{stdout}"
    );
    assert!(
        stdout.contains("ANTHROPIC_DEFAULT_HAIKU_MODEL=anthropic/claude-haiku-4.5"),
        "{stdout}"
    );
    assert!(
        stdout.contains("ANTHROPIC_DEFAULT_SONNET_MODEL=anthropic/claude-sonnet-4.6"),
        "{stdout}"
    );
    assert!(
        stdout.contains("ANTHROPIC_DEFAULT_OPUS_MODEL=anthropic/claude-opus-4.6"),
        "{stdout}"
    );
    assert!(!stdout.contains(key), "full key leaked:\n{stdout}");
}

#[test]
fn claude_dry_run_pinned_model_collapses_aliases() {
    let key = "sk-ar-v1-testkey";
    let (code, stdout, stderr) = {
        let out = anyr()
            .args([
                "claude",
                "--dry-run",
                "--yes",
                "--key",
                key,
                "--model",
                "stealth/ox-alpha",
            ])
            .env("ANYROUTER_HOME", temp_home())
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
    assert!(
        stdout.contains("ANTHROPIC_MODEL=stealth/ox-alpha[1m]"),
        "{stdout}"
    );
    // Unset alias slots follow the pinned model so nothing falls back to
    // haiku/sonnet/opus behind the user's back.
    for key_line in [
        "ANTHROPIC_DEFAULT_HAIKU_MODEL=stealth/ox-alpha[1m]",
        "ANTHROPIC_DEFAULT_SONNET_MODEL=stealth/ox-alpha[1m]",
        "ANTHROPIC_DEFAULT_OPUS_MODEL=stealth/ox-alpha[1m]",
        "CLAUDE_CODE_SUBAGENT_MODEL=stealth/ox-alpha[1m]",
    ] {
        assert!(stdout.contains(key_line), "missing {key_line}:\n{stdout}");
    }
}

#[test]
fn claude_dry_run_haiku_flag_beats_pinned_model() {
    let key = "sk-ar-v1-testkey";
    let (code, stdout, stderr) = {
        let out = anyr()
            .args([
                "claude",
                "--dry-run",
                "--yes",
                "--key",
                key,
                "--model",
                "stealth/ox-alpha",
                "--haiku",
                "z-ai/glm-4.7-flash",
            ])
            .env("ANYROUTER_HOME", temp_home())
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
    assert!(
        stdout.contains("ANTHROPIC_DEFAULT_HAIKU_MODEL=z-ai/glm-4.7-flash[1m]"),
        "{stdout}"
    );
    assert!(
        stdout.contains("ANTHROPIC_DEFAULT_SONNET_MODEL=stealth/ox-alpha[1m]"),
        "{stdout}"
    );
}

#[test]
fn claude_yolo_flag_expands_to_dangerously_skip_permissions() {
    let key = "sk-ar-v1-testkey";
    let (code, stdout, stderr) = {
        let out = anyr()
            .args(["claude", "--dry-run", "--yes", "--key", key, "--yolo"])
            .env("ANYROUTER_HOME", temp_home())
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
    assert!(
        stdout.contains("\"--dangerously-skip-permissions\""),
        "--yolo should expand to --dangerously-skip-permissions in args:\n{stdout}"
    );
}

#[test]
fn codex_yolo_flag_is_accepted_but_not_forwarded() {
    let key = "sk-ar-v1-testkey";
    let (code, stdout, stderr) = {
        let out = anyr()
            .args(["codex", "--dry-run", "--yes", "--key", key, "--yolo"])
            .env("ANYROUTER_HOME", temp_home())
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
    assert!(
        !stdout.contains("--dangerously-skip-permissions"),
        "codex has no equivalent flag; --yolo should be a no-op:\n{stdout}"
    );
}

#[test]
fn claude_dry_run_fable_flag_beats_pinned_model() {
    let key = "sk-ar-v1-testkey";
    let (code, stdout, stderr) = {
        let out = anyr()
            .args([
                "claude",
                "--dry-run",
                "--yes",
                "--key",
                key,
                "--model",
                "stealth/ox-alpha",
                "--fable",
                "anthropic/claude-fable-5",
            ])
            .env("ANYROUTER_HOME", temp_home())
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
    // Explicit --fable wins over the pinned session model...
    assert!(
        stdout.contains("ANTHROPIC_DEFAULT_FABLE_MODEL=anthropic/claude-fable-5[1m]"),
        "{stdout}"
    );
    // ...while every other unset slot still follows the pin.
    for key_line in [
        "ANTHROPIC_DEFAULT_SONNET_MODEL=stealth/ox-alpha[1m]",
        "ANTHROPIC_DEFAULT_OPUS_MODEL=stealth/ox-alpha[1m]",
    ] {
        assert!(stdout.contains(key_line), "missing {key_line}:\n{stdout}");
    }
}

#[test]
fn claude_dry_run_haiku_flag_overrides_alias() {
    let key = "sk-ar-v1-testkey";
    let (code, stdout, stderr) = {
        let out = anyr()
            .args([
                "claude",
                "--dry-run",
                "--yes",
                "--key",
                key,
                // Keep the session model on auto so this test does not depend
                // on the developer's ~/.anyrouter config default_model.
                "--model",
                "auto",
                "--haiku",
                "z-ai/glm-4.7-flash",
            ])
            .env("ANYROUTER_HOME", temp_home())
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
    assert!(
        stdout.contains("ANTHROPIC_DEFAULT_HAIKU_MODEL=z-ai/glm-4.7-flash[1m]"),
        "{stdout}"
    );
    assert!(
        stdout.contains("ANTHROPIC_DEFAULT_SONNET_MODEL=anthropic/claude-sonnet-4.6"),
        "{stdout}"
    );
}

#[test]
fn model_without_value_errors() {
    let (code, stdout, stderr) = run(&["claude", "--model"]);
    assert_ne!(code, 0);
    assert!(format!("{stdout}{stderr}").contains("requires a value"));
}

#[test]
fn launch_remembers_explicit_model_as_session_default() {
    // Launching with --model must persist it as default_model so a bare
    // `{bin} claude` next time starts with the same model. A trivially
    // successful binary stands in for claude so the spawn works everywhere.
    let stub = if cfg!(windows) { "cmd" } else { "true" };
    let home = temp_home();
    let out = anyr()
        .args([
            "claude",
            "--yes",
            "--key",
            "sk-ar-v1-testkey",
            "--model",
            "z-ai/glm-4.7-flash",
        ])
        .env("ANYROUTER_HOME", &home)
        .env("ANYROUTER_CLAUDE_PATH", stub)
        .env_remove("ANYROUTER_API_KEY")
        .output()
        .expect("launch");
    assert_eq!(out.status.code().unwrap_or(1), 0);

    // A Claude `[1m]` suffix must not be written into config (Pi/Codex 404 on it).
    let home_suffix = temp_home();
    let out = anyr()
        .args([
            "claude",
            "--yes",
            "--key",
            "sk-ar-v1-testkey",
            "--model",
            "stealth/ox-alpha[1m]",
        ])
        .env("ANYROUTER_HOME", &home_suffix)
        .env("ANYROUTER_CLAUDE_PATH", stub)
        .env_remove("ANYROUTER_API_KEY")
        .output()
        .expect("launch suffix");
    assert_eq!(out.status.code().unwrap_or(1), 0);
    let cfg_suffix =
        std::fs::read_to_string(home_suffix.join("config.yaml")).expect("config written");
    assert!(
        cfg_suffix.contains("default_model: stealth/ox-alpha"),
        "{cfg_suffix}"
    );
    assert!(
        !cfg_suffix.contains("stealth/ox-alpha[1m]"),
        "catalog id only:\n{cfg_suffix}"
    );
    let _ = std::fs::remove_dir_all(&home_suffix);

    // Second launch with no --model: dry-run reveals which model was picked.
    let out = anyr()
        .args(["claude", "--yes", "--dry-run", "--key", "sk-ar-v1-testkey"])
        .env("ANYROUTER_HOME", &home)
        .env_remove("ANYROUTER_API_KEY")
        .output()
        .expect("relaunch");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ANTHROPIC_MODEL=z-ai/glm-4.7-flash[1m]"),
        "session default not remembered:\n{stdout}"
    );

    // And the persisted config records it too.
    let cfg = std::fs::read_to_string(home.join("config.yaml")).expect("config written");
    assert!(cfg.contains("default_model: z-ai/glm-4.7-flash"), "{cfg}");
    let _ = std::fs::remove_dir_all(&home);
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
    assert!(stdout.contains("--beta"), "{stdout}");
    assert!(stdout.contains("--stable"), "{stdout}");
    assert!(
        stdout.contains("Auto-update") || stdout.contains("auto-update"),
        "upgrade help should mention auto-update, got:\n{stdout}"
    );
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
    assert!(stdout.contains("--beta"), "{stdout}");
    assert!(stdout.contains("--stable"), "{stdout}");
}

#[test]
fn update_beta_persists_channel_and_selects_prerelease() {
    let home = std::env::temp_dir().join(format!(
        "anyr-cli-update-beta-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).expect("home");
    let (code, stdout, stderr) = {
        let out = anyr()
            .args(["update", "--beta", "--check"])
            .env("ANYROUTER_HOME", &home)
            .env("ANYR_RELEASES_JSON", fixture_path())
            .env_remove("ANYR_CHANNEL")
            .output()
            .expect("update --beta --check");
        (
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("channel set to beta"), "{stdout}");
    assert!(stdout.contains("channel: beta"), "{stdout}");
    assert!(stdout.contains("latest: 0.2.0-beta.1"), "{stdout}");
    let cfg = std::fs::read_to_string(home.join("config.yaml")).expect("config");
    assert!(cfg.contains("channel: beta"), "{cfg}");
}

#[test]
fn update_stable_and_beta_conflict() {
    let (code, stdout, stderr) = run(&["update", "--beta", "--stable", "--check"]);
    assert_ne!(code, 0);
    let combined = format!("{stdout}{stderr}");
    assert!(combined.contains("either --beta or --stable"), "{combined}");
}

#[test]
fn upgrade_check_flag_is_known() {
    // Isolate from a real ~/.anyrouter whose channel would skew the check.
    let home = std::env::temp_dir().join(format!("anyr-cli-upgrade-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let fixture = std::env::temp_dir().join(format!(
        "anyr-cli-upgrade-check-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &fixture,
        r#"[{"tag_name":"v0.1.0","prerelease":false,"draft":false,"assets":[{"name":"anyr-linux-x86_64"}]}]"#,
    )
    .expect("write fixture");
    let out = anyr()
        .args(["upgrade", "--check"])
        .env("ANYR_RELEASES_JSON", &fixture)
        .env("ANYROUTER_HOME", &home)
        .env_remove("ANYR_CHANNEL")
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
    assert!(
        stdout.contains("up to date") || stdout.contains("Already up to date"),
        "{stdout}"
    );
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
    assert!(stdout.contains("latest: 0.1.99"), "{stdout}");
    assert!(stdout.contains("update available"), "{stdout}");
    assert!(
        stdout.contains("github.com/anyrouter-dev/cli/releases/download/"),
        "{stdout}"
    );
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
    assert!(stdout.contains("latest: 0.2.0-beta.1"), "{stdout}");
    assert!(stdout.contains("update available"), "{stdout}");
}

#[test]
fn login_help_describes_device_and_paste() {
    let (code, stdout, stderr) = run(&["login", "--help"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let combined = format!("{stdout}{stderr}");
    assert!(combined.contains("--device"), "{combined}");
    assert!(combined.contains("--paste"), "{combined}");
    assert!(combined.contains("device"), "{combined}");
}

#[test]
fn config_help_describes_tui() {
    let (code, stdout, stderr) = run(&["config", "--help"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("TUI") || combined.contains("Interactive"),
        "{combined}"
    );
    assert!(combined.contains("key"), "{combined}");
    assert!(combined.contains("credits"), "{combined}");
}

#[test]
fn config_no_args_non_tty_prints_status_not_picker() {
    let dir = std::env::temp_dir().join(format!("anyr-cli-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    std::fs::write(
        &path,
        "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-config-secret-value
    default_model: auto
",
    )
    .unwrap();
    let out = anyr()
        .args([
            "config",
            "--config",
            path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:9",
        ])
        .env_remove("ANYROUTER_API_KEY")
        .output()
        .expect("config");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code().unwrap_or(1), 0, "{stdout}{stderr}");
    assert!(stdout.contains("default"), "{stdout}");
    assert!(stdout.contains("api_key"), "{stdout}");
    assert!(
        !stdout.contains("sk-ar-v1-config-secret-value"),
        "full key leaked:\n{stdout}"
    );
    assert!(
        !stderr.contains("Pick 1-"),
        "non-TTY must not open picker:\n{stderr}"
    );
    assert!(
        stdout.contains("terminal") || stdout.contains("config"),
        "{stdout}"
    );
    let path_out = anyr()
        .args(["config", "path", "--config", path.to_str().unwrap()])
        .output()
        .expect("config path");
    let printed = String::from_utf8_lossy(&path_out.stdout);
    assert!(
        printed.contains(path.to_str().unwrap()),
        "config path should still print the file: {printed}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keys_and_account_help_exit_zero() {
    for cmd in ["keys", "account", "logout", "menu"] {
        let (code, stdout, stderr) = run(&[cmd, "--help"]);
        assert_eq!(code, 0, "{cmd} --help failed: {stdout}{stderr}");
    }
}

#[test]
fn account_use_switches_active_profile() {
    let dir = std::env::temp_dir().join(format!("anyr-cli-account-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    std::fs::write(
        &path,
        "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-one-secret-value
    default_model: auto
  work:
    api_key: sk-ar-v1-two-secret-value
    default_model: anthropic/claude-sonnet-4.6
",
    )
    .unwrap();
    let out = anyr()
        .args(["account", "use", "work", "--config", path.to_str().unwrap()])
        .output()
        .expect("account use");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code().unwrap_or(1), 0, "{stdout}{stderr}");
    assert!(stdout.contains("work"), "{stdout}");
    let who = anyr()
        .args(["whoami", "--config", path.to_str().unwrap()])
        .output()
        .expect("whoami");
    let who_out = String::from_utf8_lossy(&who.stdout);
    assert!(who_out.contains("work"), "{who_out}");
    assert!(
        !who_out.contains("sk-ar-v1-two-secret-value"),
        "full key leaked:\n{who_out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn models_use_persists_default_without_network_when_unknown() {
    let dir = std::env::temp_dir().join(format!("anyr-cli-models-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    std::fs::write(
        &path,
        "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-one
    default_model: auto
",
    )
    .unwrap();
    // Unknown id is rejected locally after a models fetch; without a live
    // network this should fail loudly rather than write a guess.
    let out = anyr()
        .args([
            "models",
            "use",
            "definitely-not-a-model",
            "--config",
            path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:9",
        ])
        .env_remove("ANYROUTER_API_KEY")
        .output()
        .expect("models use");
    assert_ne!(out.status.code().unwrap_or(1), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn logout_clears_key_and_does_not_print_it() {
    let dir = std::env::temp_dir().join(format!("anyr-cli-logout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    std::fs::write(
        &path,
        "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-secret-value
",
    )
    .unwrap();
    let out = anyr()
        .args(["logout", "--config", path.to_str().unwrap()])
        .output()
        .expect("logout");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code().unwrap_or(1), 0, "{combined}");
    assert!(!combined.contains("sk-ar-v1-secret-value"), "{combined}");
    let src = std::fs::read_to_string(&path).unwrap();
    assert!(!src.contains("sk-ar-v1-secret-value"), "{src}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn install_flag_is_known_on_launch() {
    let (code, stdout, stderr) = run(&[
        "claude",
        "--install",
        "--dry-run",
        "--key",
        "sk-ar-v1-testkey",
    ]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("ANTHROPIC_BASE_URL"), "{stdout}");
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

#[test]
fn upgrade_check_reads_channel_from_config() {
    let home = std::env::temp_dir().join(format!("anyr-ch-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&home);
    std::fs::write(
        home.join("config.yaml"),
        "active_profile: default\nchannel: beta\nprofiles:\n  default:\n    api_key: x\n",
    )
    .expect("write config");
    let out = anyr()
        .args(["upgrade", "--check"])
        .env("ANYR_RELEASES_JSON", fixture_path())
        .env("ANYROUTER_HOME", &home)
        .env_remove("ANYR_CHANNEL")
        .output()
        .expect("upgrade --check config channel");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code().unwrap_or(1), 0, "stderr={stderr}");
    assert!(stdout.contains("channel: beta"), "{stdout}");
    assert!(stdout.contains("latest: 0.2.0-beta.1"), "{stdout}");
}

#[test]
fn upgrade_auto_with_fixture_would_update() {
    let home = std::env::temp_dir().join(format!("anyr-auto-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&home);
    let out = anyr()
        .args(["upgrade", "--auto"])
        .env("ANYR_RELEASES_JSON", fixture_path())
        .env("ANYROUTER_HOME", &home)
        .env("ANYR_UPDATE_INTERVAL_SECS", "0")
        .env_remove("ANYR_CHANNEL")
        .output()
        .expect("upgrade --auto");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code().unwrap_or(1), 0, "stderr={stderr}");
    assert!(
        !format!("{stdout}{stderr}").contains("Unknown flag"),
        "{stdout}{stderr}"
    );
    assert!(
        stdout.contains("would update") && stdout.contains("0.1.99"),
        "auto-update should install a newer GitHub release, got:\n{stdout}"
    );
}

#[test]
fn upgrade_auto_is_quiet_when_up_to_date() {
    let home = std::env::temp_dir().join(format!("anyr-auto-current-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&home);
    let (_, ver, _) = run(&["--version"]);
    let ver = ver.trim();
    let fixture = home.join("releases.json");
    std::fs::write(
        &fixture,
        format!(
            r#"[{{"tag_name":"v{ver}","prerelease":false,"assets":[{{"name":"anyr-linux-x86_64","browser_download_url":"https://github.com/anyrouter-dev/cli/releases/download/v{ver}/anyr-linux-x86_64"}}]}}]"#
        ),
    )
    .expect("write fixture");
    let out = anyr()
        .args(["upgrade", "--auto"])
        .env("ANYR_RELEASES_JSON", &fixture)
        .env("ANYROUTER_HOME", &home)
        .env("ANYR_UPDATE_INTERVAL_SECS", "0")
        .output()
        .expect("upgrade --auto current");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code().unwrap_or(1), 0, "stderr={stderr}");
    assert!(
        !stdout.contains("would update") && !stdout.contains("update available"),
        "up-to-date auto-update should be silent, got:\n{stdout}"
    );
}

#[test]
fn menu_dump_tui_prints_plain_frame() {
    let dir = std::env::temp_dir().join(format!("anyr-cli-menu-dump-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    std::fs::write(
        &path,
        "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-menu-dump-secret-value-abcdef
    default_model: auto
",
    )
    .unwrap();
    let out = anyr()
        .args(["menu", "--dump-tui", "--config", path.to_str().unwrap()])
        .env("ANYR_AGENTS", "claude,codex")
        .output()
        .expect("menu dump");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code().unwrap_or(1), 0, "{stdout}{stderr}");
    assert!(
        !stdout.contains('\u{1b}'),
        "dump must be ANSI-free: {stdout}"
    );
    // Palette frame: input line, grouped launch rows, configure rows.
    assert!(stdout.contains("▲ anyr"), "{stdout}");
    assert!(stdout.contains("LAUNCH"), "{stdout}");
    assert!(stdout.contains("claude"), "{stdout}");
    assert!(stdout.contains("CONFIGURE"), "{stdout}");
    assert!(stdout.contains("config…"), "{stdout}");
    assert!(stdout.contains("account…"), "{stdout}");
    assert!(stdout.contains("key…"), "{stdout}");
    assert!(stdout.contains("model…"), "{stdout}");
    assert!(stdout.contains("install…"), "{stdout}");
    assert!(stdout.contains("quit"), "{stdout}");
    assert!(
        stdout.contains('❯'),
        "palette must show the input line: {stdout}"
    );
    assert!(
        stdout.contains('╭') && stdout.contains('╯'),
        "dump should look like a dialog card: {stdout}"
    );
    assert!(
        !stdout.contains("menu-dump-secret-value"),
        "dump must not leak full secret: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn menu_dump_tui_empty_agents_shows_install() {
    let dir = std::env::temp_dir().join(format!("anyr-cli-menu-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    std::fs::write(
        &path,
        "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-empty-agents-secret-abcdef
    default_model: auto
",
    )
    .unwrap();
    let out = anyr()
        .args(["menu", "--dump-tui", "--config", path.to_str().unwrap()])
        .env("ANYR_AGENTS", "none")
        .output()
        .expect("menu dump empty");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code().unwrap_or(1), 0, "{stdout}{stderr}");
    assert!(stdout.contains("install an agent…"), "{stdout}");
    assert!(stdout.contains("none detected"), "{stdout}");
    assert!(!stdout.contains("◆ claude"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_dump_tui_prints_plain_frame() {
    let dir = std::env::temp_dir().join(format!("anyr-cli-config-dump-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    std::fs::write(
        &path,
        "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-config-dump-secret-value-abcdef
    default_model: auto
",
    )
    .unwrap();
    let out = anyr()
        .args(["config", "--dump-tui", "--config", path.to_str().unwrap()])
        .output()
        .expect("config dump");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code().unwrap_or(1), 0, "{stdout}{stderr}");
    assert!(
        !stdout.contains('\u{1b}'),
        "dump must be ANSI-free: {stdout}"
    );
    assert!(
        stdout.contains('╭') && stdout.contains('╯'),
        "dialog card: {stdout}"
    );
    // Grouped sections with current values.
    for section in ["ACCOUNT", "MODEL", "AGENT", "GENERAL"] {
        assert!(stdout.contains(section), "missing {section} in:\n{stdout}");
    }
    let dumped: Vec<&str> = stdout.lines().collect();
    let acct = dumped
        .iter()
        .position(|l| l.contains("ACCOUNT"))
        .expect("ACCOUNT");
    let model = dumped
        .iter()
        .position(|l| l.contains("MODEL"))
        .expect("MODEL");
    assert!(
        model > acct + 1,
        "expected padding between ACCOUNT and MODEL:\n{stdout}"
    );
    for row in [
        "account",
        "api key",
        "default",
        "coding agent",
        "auto-update",
        "update channel",
        "[general]",
        "claude",
        "codex",
    ] {
        assert!(stdout.contains(row), "missing row \"{row}\" in:\n{stdout}");
    }
    assert!(
        !stdout.contains("config-dump-secret-value"),
        "dump must not leak full secret: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn menu_help_mentions_dump_tui() {
    let (code, stdout, stderr) = run(&["menu", "--help"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("dump-tui") || combined.contains("Ratatui"),
        "{combined}"
    );
}
