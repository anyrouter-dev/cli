//! Hermetic e2e for MCP auto-connect: the built `anyr` launches
//! tests/fake-mcp-agent.py in place of each agent, against a local gateway
//! that serves `/api/v1/mcp/config` and a tiny MCP server. The fake agent
//! reads the MCP config the way the real agent does and lists the tools, so
//! these tests assert what reaches the server, not what anyr prints.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

const KEY: &str = "sk-ar-v1-mcpautotestkey0123456789";
const CATALOG: &str = r#"{"object":"list","data":[{"id":"z-ai/glm-4.7","owned_by":"z-ai","name":"GLM 4.7","category":"text","capabilities":["chat"],"created":100,"context_length":204800}]}"#;

#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    auth: String,
}

struct Gateway {
    base: String,
    seen: Arc<Mutex<Vec<Req>>>,
}

impl Gateway {
    /// `enabled`: what `GET /api/v1/mcp/config` answers for this workspace.
    fn start(enabled: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/api", listener.local_addr().unwrap());
        let seen: Arc<Mutex<Vec<Req>>> = Arc::default();
        let seen_in = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let seen = Arc::clone(&seen_in);
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() {
                        return;
                    }
                    let (mut auth, mut len) = (String::new(), 0usize);
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                        let lower = h.to_ascii_lowercase();
                        if let Some(v) = lower.strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                        if lower.starts_with("authorization:") {
                            auth = h["authorization:".len()..].trim().to_string();
                        }
                    }
                    let mut body = vec![0u8; len];
                    let _ = reader.read_exact(&mut body);
                    let mut parts = line.split_whitespace();
                    let req = Req {
                        method: parts.next().unwrap_or("").into(),
                        path: parts.next().unwrap_or("").into(),
                        auth,
                    };
                    let (status, out) = route(&req, &body, enabled);
                    seen.lock().unwrap().push(req);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                        out.len()
                    );
                });
            }
        });
        Gateway { base, seen }
    }

    fn hits(&self, method: &str, path: &str) -> Vec<Req> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == method && r.path.split('?').next() == Some(path))
            .cloned()
            .collect()
    }
}

fn route(req: &Req, body: &[u8], enabled: bool) -> (u16, String) {
    let path = req.path.split('?').next().unwrap_or("");
    let authed = req.auth == format!("Bearer {KEY}");
    match (req.method.as_str(), path) {
        ("GET", "/api/v1/models") => (200, CATALOG.into()),
        ("GET", "/api/v1/credits") => (
            200,
            r#"{"data":{"total_credits":1,"total_usage":0}}"#.into(),
        ),
        ("GET", "/api/v1/mcp/config") if authed => (
            200,
            json!({"enabled": enabled, "setting": null, "default": enabled,
                   "url": "ignored-by-the-cli", "can_edit": false})
            .to_string(),
        ),
        ("POST", "/api/v1/mcp") if authed => {
            let rpc: Value = serde_json::from_slice(body).unwrap_or_default();
            let result = match rpc["method"].as_str() {
                Some("tools/list") => {
                    json!({"tools": [{"name": "list_models"}, {"name": "get_system_status"}]})
                }
                _ => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}}),
            };
            (
                200,
                json!({"jsonrpc": "2.0", "id": rpc["id"], "result": result}).to_string(),
            )
        }
        (_, p) if p.starts_with("/api/v1/mcp") => (401, r#"{"error":"unauthorized"}"#.into()),
        _ => (404, r#"{"error":{"message":"not found"}}"#.into()),
    }
}

fn temp_home(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("anyr-mcpauto-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn anyr(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_anyr"));
    cmd.env("ANYR_NO_UPDATE", "1")
        .env("ANYR_NO_CATALOG", "1")
        .env("NO_COLOR", "1")
        .env("ANYROUTER_HOME", home)
        .env("HOME", home)
        .env("GROK_HOME", home.join("grok"))
        .env_remove("ANYROUTER_API_KEY")
        .env_remove("ANYROUTER_MANAGEMENT_KEY")
        .env_remove("ANYROUTER_PROFILE")
        .env_remove("OPENCODE_CONFIG_CONTENT")
        .stdin(Stdio::null());
    cmd
}

fn run(cmd: &mut Command) -> (i32, String, String) {
    let out = cmd.output().expect("spawn anyr");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn login(home: &Path, base: &str) {
    let mut child = anyr(home)
        .args(["login", "--key", "-", "--base-url", base])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{KEY}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "login failed: {out:?}");
}

/// `anyr <agent> [extra]` with the fake agent; returns its stdout.
fn launch(gw: &Gateway, home: &Path, agent: &str, extra: &[&str]) -> String {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake-mcp-agent.py");
    let (code, out, err) = run(anyr(home)
        .args([agent, "--yes", "--model", "z-ai/glm-4.7"])
        .args(extra)
        .env(
            format!("ANYROUTER_{}_PATH", agent.to_ascii_uppercase()),
            &fake,
        )
        .env("FAKE_AGENT", agent));
    assert_eq!(code, 0, "anyr {agent} failed:\nstdout:{out}\nstderr:{err}");
    assert!(
        !out.contains(KEY) && !err.contains(KEY),
        "{agent} printed the key"
    );
    let _ = gw;
    out
}

#[test]
fn every_auto_agent_lists_the_gateway_tools_with_the_launch_key() {
    for agent in ["claude", "codex", "opencode", "grok"] {
        let gw = Gateway::start(true);
        let home = temp_home(agent);
        login(&home, &gw.base);
        let out = launch(&gw, &home, agent, &[]);
        assert!(
            out.contains("TOOLS list_models,get_system_status"),
            "{agent} did not reach the MCP tools:\n{out}"
        );
        let mcp = gw.hits("POST", "/api/v1/mcp");
        assert_eq!(mcp.len(), 2, "{agent}: {mcp:?}");
        assert!(mcp.iter().all(|r| r.auth == format!("Bearer {KEY}")));
    }
}

#[test]
fn a_workspace_with_auto_connect_off_gets_nothing() {
    for agent in ["claude", "codex", "opencode", "grok"] {
        let gw = Gateway::start(false);
        let home = temp_home(&format!("{agent}-off"));
        login(&home, &gw.base);
        assert_eq!(launch(&gw, &home, agent, &[]).trim(), "NO_MCP", "{agent}");
        assert_eq!(gw.hits("GET", "/api/v1/mcp/config").len(), 1, "{agent}");
    }
}

#[test]
fn no_mcp_and_mcp_auto_off_skip_the_lookup_entirely() {
    let gw = Gateway::start(true);
    let home = temp_home("optout");
    login(&home, &gw.base);
    assert_eq!(launch(&gw, &home, "claude", &["--no-mcp"]).trim(), "NO_MCP");
    assert!(gw.hits("GET", "/api/v1/mcp/config").is_empty());

    let (code, out, _) = run(anyr(&home).args(["mcp", "auto", "off"]));
    assert_eq!(code, 0);
    assert!(out.contains("off"), "{out}");
    assert_eq!(launch(&gw, &home, "codex", &[]).trim(), "NO_MCP");
    assert!(gw.hits("GET", "/api/v1/mcp/config").is_empty());

    run(anyr(&home).args(["mcp", "auto", "on"]));
    assert!(launch(&gw, &home, "codex", &[]).contains("TOOLS"));
}

#[test]
fn grok_entry_follows_the_setting_and_mcp_remove_cleans_it() {
    let gw = Gateway::start(true);
    let home = temp_home("grok-cycle");
    login(&home, &gw.base);
    let grok_config = home.join("grok").join("config.toml");
    launch(&gw, &home, "grok", &[]);
    let text = std::fs::read_to_string(&grok_config).unwrap();
    assert!(
        text.contains("# Managed by anyr") && !text.contains(KEY),
        "{text}"
    );

    // Turning it off on the next launch takes anyr's entry back out.
    assert_eq!(launch(&gw, &home, "grok", &["--no-mcp"]).trim(), "NO_MCP");
    assert!(!std::fs::read_to_string(&grok_config)
        .unwrap()
        .contains("anyrouter"));

    run(anyr(&home).args(["mcp", "add", "grok"]));
    assert!(std::fs::read_to_string(&grok_config)
        .unwrap()
        .contains("[mcp_servers.anyrouter]"));
    let (code, out, _) = run(anyr(&home).args(["mcp", "remove", "grok"]));
    assert_eq!(code, 0, "{out}");
    assert!(!std::fs::read_to_string(&grok_config)
        .unwrap()
        .contains("anyrouter"));
}

#[test]
fn dry_run_shows_the_wiring_with_the_key_redacted() {
    let gw = Gateway::start(true);
    let home = temp_home("dry");
    login(&home, &gw.base);
    let (code, out, err) = run(anyr(&home).args(["claude", "--yes", "--dry-run"]));
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("--mcp-config="), "{out}");
    assert!(out.contains("mcp: anyrouter http://127.0.0.1"), "{out}");
    assert!(!out.contains(KEY), "dry-run printed the key:\n{out}");

    let (_, out, _) = run(anyr(&home).args(["pi", "--yes", "--dry-run"]));
    assert!(out.contains("mcp: off (Pi has no MCP support)"), "{out}");
}
