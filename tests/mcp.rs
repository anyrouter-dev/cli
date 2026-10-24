//! `anyr mcp` against a temp HOME and a local scripted server.

mod common;
use common::{serve, KEY};
use std::path::Path;
use std::process::Command;

const URL: &str = "https://anyrouter.dev/api/v1/mcp";

fn anyr(home: &Path, args: &[&str]) -> (i32, String, String) {
    anyr_env(home, args, &[])
}

fn anyr_env(home: &Path, args: &[&str], extra: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_anyr"));
    cmd.env("ANYR_NO_UPDATE", "1")
        .env("NO_COLOR", "1")
        .env("ANYROUTER_API_KEY", KEY)
        .env_remove("ANYROUTER_MANAGEMENT_KEY")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CODEX_HOME")
        .env("ANYROUTER_HOME", home.join(".anyrouter"))
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(args);
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run anyr");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn temp_home(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("anyr-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn url_prints_the_public_endpoint() {
    let home = temp_home("url");
    let (code, out, _) = anyr(&home, &["mcp", "url"]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), URL);
}

#[test]
fn claude_print_redacts_the_key_unless_show_key() {
    let home = temp_home("claude-print");
    let (code, out, _) = anyr(&home, &["mcp", "add", "claude", "--print"]);
    assert_eq!(code, 0);
    assert_eq!(
        out.trim(),
        format!("claude mcp add --transport http anyrouter {URL} --header \"Authorization: Bearer $ANYROUTER_API_KEY\"")
    );
    assert!(!out.contains(KEY));
    let (_, out, _) = anyr(&home, &["mcp", "add", "claude", "--print", "--show-key"]);
    assert!(out.contains(&format!("Bearer {KEY}")));
}

#[test]
fn print_changes_nothing_for_every_file_agent() {
    let home = temp_home("print");
    for agent in ["codex", "opencode", "cursor"] {
        let (code, out, err) = anyr(&home, &["mcp", "add", agent, "--print"]);
        assert_eq!(code, 0, "{agent}: {err}");
        assert!(out.contains(URL), "{agent}: {out}");
        assert!(!out.contains(KEY), "{agent} leaked the key");
    }
    assert_eq!(
        std::fs::read_dir(&home).unwrap().count(),
        0,
        "--print wrote files"
    );
}

#[test]
fn opencode_write_is_valid_keeps_user_content_and_is_idempotent() {
    let home = temp_home("opencode");
    let path = home.join(".config/opencode/opencode.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"theme":"dark","mcp":{"other":{"type":"local","command":["x"]}}}"#,
    )
    .unwrap();

    let (code, _, err) = anyr(&home, &["mcp", "add", "opencode"]);
    assert_eq!(code, 0, "{err}");
    let first = std::fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&first).expect("valid JSON");
    assert_eq!(v["theme"], "dark");
    assert_eq!(v["mcp"]["other"]["command"][0], "x");
    assert_eq!(v["mcp"]["anyrouter"]["type"], "remote");
    assert_eq!(v["mcp"]["anyrouter"]["url"], URL);
    assert_eq!(
        v["mcp"]["anyrouter"]["headers"]["Authorization"],
        "Bearer {env:ANYROUTER_API_KEY}"
    );
    assert!(!first.contains(KEY));

    let (code, out, _) = anyr(&home, &["mcp", "add", "opencode"]);
    assert_eq!(code, 0);
    assert!(out.contains("nothing changed"), "{out}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
}

#[test]
fn opencode_refuses_to_rewrite_a_commented_config() {
    let home = temp_home("opencode-jsonc");
    let dir = home.join(".config/opencode");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("opencode.jsonc"), "{ // mine\n}").unwrap();
    let (code, _, err) = anyr(&home, &["mcp", "add", "opencode"]);
    assert_eq!(code, 1);
    assert!(err.contains("opencode.jsonc"), "{err}");
    assert!(!dir.join("opencode.json").exists());
}

#[test]
fn invalid_json_is_an_error_and_the_file_is_untouched() {
    let home = temp_home("bad-json");
    let path = home.join(".cursor/mcp.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ not json").unwrap();
    let (code, _, err) = anyr(&home, &["mcp", "add", "cursor"]);
    assert_eq!(code, 1);
    assert!(err.starts_with("error:"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
}

#[test]
fn cursor_write_uses_env_interpolation_and_show_key_writes_the_key() {
    let home = temp_home("cursor");
    let (code, _, err) = anyr(&home, &["mcp", "add", "cursor"]);
    assert_eq!(code, 0, "{err}");
    let path = home.join(".cursor/mcp.json");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        v["mcpServers"]["anyrouter"]["headers"]["Authorization"],
        "Bearer ${env:ANYROUTER_API_KEY}"
    );
    let (code, _, _) = anyr(&home, &["mcp", "add", "cursor", "--show-key"]);
    assert_eq!(code, 0);
    assert!(std::fs::read_to_string(&path).unwrap().contains(KEY));
}

#[test]
fn codex_write_preserves_config_and_is_idempotent() {
    let home = temp_home("codex");
    let path = home.join(".codex/config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "model = \"gpt-5\"\n\n[mcp_servers.other]\nurl = \"https://o\"\n",
    )
    .unwrap();
    let (code, _, err) = anyr(&home, &["mcp", "add", "codex"]);
    assert_eq!(code, 0, "{err}");
    let first = std::fs::read_to_string(&path).unwrap();
    assert!(first.starts_with("model = \"gpt-5\"\n\n[mcp_servers.other]\nurl = \"https://o\"\n"));
    assert!(first.contains(&format!(
        "[mcp_servers.anyrouter]\nurl = \"{URL}\"\nbearer_token_env_var = \"ANYROUTER_API_KEY\"\n"
    )));
    assert!(!first.contains(KEY));
    anyr(&home, &["mcp", "add", "codex"]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
}

#[cfg(unix)]
#[test]
fn claude_runs_the_claude_binary_with_the_header() {
    use std::os::unix::fs::PermissionsExt;
    let home = temp_home("claude-run");
    let log = home.join("args.txt");
    let fake = home.join("fake-claude");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (code, _, err) = anyr_env(
        &home,
        &["mcp", "add", "claude"],
        &[("ANYROUTER_CLAUDE_PATH", fake.to_str().unwrap())],
    );
    assert_eq!(code, 0, "{err}");
    let got: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(
        got,
        [
            "mcp",
            "add",
            "--transport",
            "http",
            "anyrouter",
            URL,
            "--header",
            &format!("Authorization: Bearer {KEY}")
        ]
    );
}

#[test]
fn missing_claude_binary_is_a_clear_error() {
    let home = temp_home("claude-missing");
    let (code, _, err) = anyr_env(
        &home,
        &["mcp", "add", "claude"],
        &[("ANYROUTER_CLAUDE_PATH", home.join("nope").to_str().unwrap())],
    );
    assert_eq!(code, 1);
    assert!(
        err.contains("was not found") && err.contains("--print"),
        "{err}"
    );
}

#[test]
fn unknown_agent_and_bad_usage_exit_2() {
    let home = temp_home("usage");
    let (code, _, err) = anyr(&home, &["mcp", "add", "emacs"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("claude|codex|opencode|cursor"), "{err}");
    assert_eq!(anyr(&home, &["mcp"]).0, 2);
}

#[test]
fn connections_lists_rows_and_json_is_raw() {
    let body = r#"{"data":[{"slug":"github","name":"GitHub","status":"connected","auth_health":"healthy","tool_count":12,"recent_call_count":7,"recent_error_count":1,"server_url":"https://mcp.example/x"}]}"#;
    let (base, rx) = serve(vec![(200, body.into()), (200, body.into())]);
    let home = temp_home("conn");
    let (code, out, err) = anyr(&home, &["mcp", "connections", "--base-url", &base]);
    assert_eq!(code, 0, "{err}");
    let seen = rx.recv().unwrap();
    assert_eq!(
        (seen.method.as_str(), seen.path.as_str()),
        ("GET", "/api/v1/connections")
    );
    assert_eq!(seen.auth, format!("Bearer {KEY}"));
    for want in [
        "github",
        "GitHub",
        "connected",
        "healthy",
        "12",
        "7",
        "https://mcp.example/x",
    ] {
        assert!(out.contains(want), "missing {want}: {out}");
    }
    let (_, out, _) = anyr(
        &home,
        &["mcp", "connections", "--json", "--base-url", &base],
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["data"][0]["slug"], "github");
}

#[test]
fn connections_error_goes_to_stderr_with_exit_1() {
    let (base, _rx) = serve(vec![(
        401,
        r#"{"error":{"message":"Unauthorized"}}"#.into(),
    )]);
    let home = temp_home("conn-err");
    let (code, out, err) = anyr(&home, &["mcp", "connections", "--base-url", &base]);
    assert_eq!(code, 1);
    assert!(out.is_empty());
    assert!(err.contains("Unauthorized"), "{err}");
}
