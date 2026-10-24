//! `anyr <agent> [flags]`: anyr keeps its own flags, maps `--yolo` to the
//! agent's native flag, and hands every other flag to the agent in order.

use std::process::Command;

const KEY: &str = "sk-ar-v1-fixture-key-0001";

/// Agent, and its native skip-permission argv (empty: none).
const AGENTS: &[(&str, &[&str])] = &[
    ("claude", &["--dangerously-skip-permissions"]),
    ("codex", &["--dangerously-bypass-approvals-and-sandbox"]),
    ("grok", &["--always-approve"]),
    ("opencode", &["--auto"]),
    ("pi", &[]),
    ("pool", &["--unsafe-auto-allow"]),
];

struct DryRun {
    code: i32,
    args: Vec<String>,
    stdout: String,
    stderr: String,
}

/// `anyr <agent> <anyr flags> <extra...>` in dry-run, with a nonexistent
/// agent path so OpenCode's version probe cannot add `--standalone`.
fn dry_run(agent: &str, extra: &[&str]) -> DryRun {
    let home = std::env::temp_dir().join(format!(
        "anyr-passthrough-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_anyr"))
        .args([
            agent,
            "--dry-run",
            "--yes",
            "--no-mcp",
            "--key",
            KEY,
            "--command-path",
            "/nonexistent/agent-bin",
        ])
        .args(extra)
        .env("ANYROUTER_HOME", &home)
        .env("ANYR_NO_UPDATE", "1")
        .env("ANYR_NO_CATALOG", "1")
        .env_remove("ANYROUTER_API_KEY")
        .output()
        .expect("spawn anyr");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let args = stdout
        .lines()
        .find_map(|l| l.strip_prefix("args: "))
        .map(|json| serde_json::from_str(json).expect("args line is JSON"))
        .unwrap_or_default();
    DryRun {
        code: out.status.code().unwrap_or(1),
        args,
        stdout,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn count_run(haystack: &[String], needle: &[&str]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| w.iter().zip(needle).all(|(a, b)| a == b))
        .count()
}

#[test]
fn unknown_flags_reach_the_agent_in_order_without_separator() {
    for (agent, _) in AGENTS {
        let base = dry_run(agent, &[]);
        assert_eq!(base.code, 0, "{agent}: {}", base.stderr);
        let extra = ["--no-skills", "-x", "--tools", "read,grep", "--no-tools"];
        let got = dry_run(agent, &extra);
        assert_eq!(got.code, 0, "{agent}: {}", got.stderr);
        let mut want = base.args.clone();
        want.extend(strs(&extra));
        assert_eq!(got.args, want, "{agent}");
    }
}

#[test]
fn equals_form_unknown_flag_is_forwarded_not_parsed_as_bool() {
    let got = dry_run("pi", &["--tools=read,grep"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert_eq!(
        got.args.last().map(String::as_str),
        Some("--tools=read,grep")
    );
}

#[test]
fn yolo_maps_to_each_agents_native_flag() {
    for (agent, native) in AGENTS {
        let got = dry_run(agent, &["--yolo", "--continue"]);
        assert_eq!(got.code, 0, "{agent}: {}", got.stderr);
        assert!(
            !got.args.iter().any(|a| a == "--yolo"),
            "{agent}: {:?}",
            got.args
        );
        if native.is_empty() {
            assert!(
                got.stderr
                    .contains("pi has no flag to skip permission prompts"),
                "{agent} must say --yolo was dropped: {}",
                got.stderr
            );
            assert_eq!(got.args, dry_run(agent, &["--continue"]).args, "{agent}");
        } else {
            assert_eq!(count_run(&got.args, native), 1, "{agent}: {:?}", got.args);
            assert_eq!(got.args.last().map(String::as_str), Some("--continue"));
        }
    }
}

#[test]
fn native_flag_passed_directly_is_not_doubled_by_yolo() {
    for (agent, native) in AGENTS.iter().filter(|(_, n)| !n.is_empty()) {
        let mut extra = native.to_vec();
        extra.push("--yolo");
        let got = dry_run(agent, &extra);
        assert_eq!(got.code, 0, "{agent}: {}", got.stderr);
        assert_eq!(count_run(&got.args, native), 1, "{agent}: {:?}", got.args);
    }
}

#[test]
fn separator_sends_anyr_flag_names_to_the_agent_raw() {
    for (agent, native) in AGENTS {
        let base = dry_run(agent, &[]);
        let raw = ["--model", "raw/agent-model", "--yolo", "--dry-run"];
        let mut extra = vec!["--"];
        extra.extend(raw);
        let got = dry_run(agent, &extra);
        assert_eq!(got.code, 0, "{agent}: {}", got.stderr);
        let mut want = base.args.clone();
        want.extend(strs(&raw));
        assert_eq!(got.args, want, "{agent}: raw tail, no yolo mapping");
        if let Some(first) = native.first() {
            assert!(!got.args.iter().any(|a| a == first), "{agent}");
        }
        let anyr_side: String = got
            .stdout
            .lines()
            .filter(|l| !l.starts_with("args: "))
            .collect();
        assert!(
            !anyr_side.contains("raw/agent-model"),
            "{agent}: anyr took the raw --model:\n{}",
            got.stdout
        );
    }
}

#[test]
fn anyr_model_flag_is_still_consumed() {
    for (agent, _) in AGENTS {
        let with_model = dry_run(agent, &["--model", "test-owner/m1"]);
        assert_eq!(with_model.code, 0, "{agent}: {}", with_model.stderr);
        assert!(
            with_model.stdout.contains("test-owner/m1"),
            "{agent} never got the model:\n{}",
            with_model.stdout
        );
        let mixed = dry_run(agent, &["--no-skills", "--model", "test-owner/m1", "-v"]);
        let mut want = with_model.args.clone();
        want.extend(strs(&["--no-skills", "-v"]));
        assert_eq!(mixed.args, want, "{agent}");
    }
}

#[test]
fn claude_only_flags_go_to_other_agents() {
    let got = dry_run("codex", &["--haiku", "x"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(
        got.args.ends_with(&strs(&["--haiku", "x"])),
        "{:?}",
        got.args
    );
    let claude = dry_run("claude", &["--haiku", "test-owner/h"]);
    assert!(
        !claude.args.iter().any(|a| a == "--haiku"),
        "{:?}",
        claude.args
    );
    assert!(claude.stdout.contains("test-owner/h"), "{}", claude.stdout);
}

#[test]
fn non_launcher_commands_still_reject_unknown_flags() {
    let out = Command::new(env!("CARGO_BIN_EXE_anyr"))
        .args(["usage", "--no-skills"])
        .env("ANYR_NO_UPDATE", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Unknown flag --no-skills"));
}

#[test]
fn yolo_follows_the_subcommand_the_flag_belongs_to() {
    let opencode = dry_run("opencode", &["run", "--yolo", "fix it"]);
    assert_eq!(opencode.code, 0, "{}", opencode.stderr);
    assert!(
        opencode.args.ends_with(&strs(&["run", "--auto", "fix it"])),
        "{:?}",
        opencode.args
    );
    let codex = dry_run("codex", &["exec", "--yolo", "fix it"]);
    assert!(
        codex.args.ends_with(&strs(&[
            "--dangerously-bypass-approvals-and-sandbox",
            "exec",
            "fix it"
        ])),
        "{:?}",
        codex.args
    );
}

#[test]
fn cc_alias_forwards_like_claude() {
    let got = dry_run("cc", &["--no-skills", "--yolo"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert_eq!(
        got.args,
        strs(&["--dangerously-skip-permissions", "--no-skills"])
    );
}

#[test]
fn pool_yolo_passes_unsafe_auto_allow_and_not_yolo() {
    let got = dry_run("pool", &["--yolo", "--dry-run"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert_eq!(
        count_run(&got.args, &["--unsafe-auto-allow"]),
        1,
        "{:?}",
        got.args
    );
    assert!(
        !got.args
            .iter()
            .any(|a| a == "--yolo" || a == "always-allow"),
        "{:?}",
        got.args
    );
}

#[test]
fn json_and_quiet_reach_the_agent_on_launchers() {
    let codex = dry_run("codex", &["exec", "--json", "x"]);
    assert_eq!(codex.code, 0, "{}", codex.stderr);
    assert!(
        codex.args.ends_with(&strs(&["exec", "--json", "x"])),
        "{:?}",
        codex.args
    );
    let claude = dry_run("claude", &["--quiet", "-p", "x"]);
    assert_eq!(claude.code, 0, "{}", claude.stderr);
    assert!(
        claude.args.ends_with(&strs(&["--quiet", "-p", "x"])),
        "{:?}",
        claude.args
    );
}

#[test]
fn pool_keeps_its_documented_json_flag() {
    let got = dry_run("pool", &["--json"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(!got.args.iter().any(|a| a == "--json"), "{:?}", got.args);
}

#[test]
fn non_launcher_unknown_flag_still_exits_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_anyr"))
        .args(["models", "--bogus"])
        .env("ANYR_NO_UPDATE", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn opencode_top_level_yolo_leads_the_args() {
    // opencode 2.0.24 `--help` lists `--auto` on the root command too.
    let base = dry_run("opencode", &["fix it"]).args;
    let got = dry_run("opencode", &["--yolo", "fix it"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    let mut want = strs(&["--auto"]);
    want.extend(base);
    assert_eq!(got.args, want);
}

#[test]
fn skills_off_and_tools_off_map_to_pi_native_flags() {
    let got = dry_run("pi", &["--skills-off", "--tools-off"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    let base = dry_run("pi", &[]).args;
    assert_eq!(
        &got.args[..got.args.len() - base.len()],
        ["--no-skills", "--no-tools"],
        "{:?}",
        got.args
    );
    assert!(
        !got.args.iter().any(|a| a.ends_with("-off")),
        "{:?}",
        got.args
    );
    let native = dry_run("pi", &["--no-skills"]);
    assert_eq!(native.args.last().map(String::as_str), Some("--no-skills"));
}

#[test]
fn skills_off_leads_a_claude_subcommand_and_a_literal_separator() {
    let base = dry_run("claude", &[]).args;
    let sub = dry_run("claude", &["--skills-off", "mcp", "list"]);
    assert_eq!(sub.code, 0, "{}", sub.stderr);
    let mut want = strs(&["--disable-slash-commands"]);
    want.extend(base.clone());
    want.extend(strs(&["mcp", "list"]));
    assert_eq!(sub.args, want);
    let sep = dry_run("claude", &["--skills-off", "--", "-p", "hi"]);
    let mut want = strs(&["--disable-slash-commands"]);
    want.extend(base);
    want.extend(strs(&["--", "-p", "hi"]));
    assert_eq!(sep.args, want, "{:?}", sep.args);
}

#[test]
fn skills_off_is_dropped_with_a_note_where_the_agent_has_no_switch() {
    let got = dry_run("codex", &["--skills-off"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(
        !got.args.iter().any(|a| a == "--skills-off"),
        "{:?}",
        got.args
    );
    assert!(
        got.stderr
            .contains("codex has no skills switch; ignoring --skills-off"),
        "{}",
        got.stderr
    );
    let claude = dry_run("claude", &["--skills-off"]);
    assert_eq!(
        claude.args.first().map(String::as_str),
        Some("--disable-slash-commands")
    );
}

#[test]
fn skills_off_after_separator_stays_raw() {
    let base = dry_run("pi", &[]).args;
    let got = dry_run("pi", &["--", "--skills-off", "--tools-off"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    let mut want = base;
    want.extend(strs(&["--skills-off", "--tools-off"]));
    assert_eq!(got.args, want);
}

#[test]
fn skills_off_equals_true_maps_and_never_rewrites_a_value() {
    let got = dry_run("pi", &["--skills-off=true"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert_eq!(got.args.first().map(String::as_str), Some("--no-skills"));
    let value = dry_run("pi", &["-p", "--", "--skills-off"]);
    assert_eq!(value.args.last().map(String::as_str), Some("--skills-off"));
}
