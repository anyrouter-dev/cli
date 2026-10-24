//! Hermetic end-to-end tests: the built `anyr` binary against local mock
//! servers, with no network beyond loopback. Each test walks one user path
//! (sign in, chat, launch an agent, run the relay) through the real binary
//! and asserts what reaches the server, not what the CLI prints about it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

const KEY: &str = "sk-ar-v1-e2etestkey0123456789abcdef";

/// One request the mock saw.
#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    auth: String,
    body: String,
}

type Handler = dyn Fn(&Req) -> (u16, &'static str, String) + Send + Sync;

/// HTTP mock that answers every connection through `handler` and records it.
struct Mock {
    base: String,
    seen: Arc<Mutex<Vec<Req>>>,
}

impl Mock {
    fn start(
        handler: impl Fn(&Req) -> (u16, &'static str, String) + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/api", listener.local_addr().unwrap());
        let seen: Arc<Mutex<Vec<Req>>> = Arc::default();
        let seen_in = Arc::clone(&seen);
        let handler: Arc<Handler> = Arc::new(handler);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let seen = Arc::clone(&seen_in);
                let handler = Arc::clone(&handler);
                std::thread::spawn(move || {
                    let Some(req) = read_request(&mut stream) else {
                        return;
                    };
                    let (status, ctype, body) = handler(&req);
                    seen.lock().unwrap().push(req);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                });
            }
        });
        Mock { base, seen }
    }

    fn seen(&self) -> Vec<Req> {
        self.seen.lock().unwrap().clone()
    }

    /// Requests whose path contains `needle`.
    fn hits(&self, needle: &str) -> Vec<Req> {
        self.seen()
            .into_iter()
            .filter(|r| r.path.contains(needle))
            .collect()
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Req> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let (mut auth, mut len) = (String::new(), 0usize);
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).ok()? == 0 || h.trim().is_empty() {
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
    reader.read_exact(&mut body).ok()?;
    let mut parts = line.split_whitespace();
    Some(Req {
        method: parts.next().unwrap_or("").into(),
        path: parts.next().unwrap_or("").into(),
        auth,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

const CATALOG: &str = r#"{"object":"list","data":[{"id":"z-ai/glm-4.7","owned_by":"z-ai","name":"GLM 4.7","category":"text","capabilities":["chat"],"created":100,"context_length":204800,"pricing":{"input_per_1m":0.14,"output_per_1m":0.28}},{"id":"nvidia/free-a","owned_by":"nvidia","name":"Free A","category":"text","capabilities":["chat"],"created":300,"context_length":128000,"pricing":{"input_per_1m":0,"output_per_1m":0,"billing":"free"}}]}"#;
const CHAT_SSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"pon\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"g\"}}]}\n\ndata: [DONE]\n\n";

/// A gateway that knows the routes the CLI calls on a normal day.
fn gateway() -> Mock {
    Mock::start(|r| {
        let path = r.path.split('?').next().unwrap_or("");
        match (r.method.as_str(), path) {
            ("GET", "/api/v1/models") => (200, "application/json", CATALOG.into()),
            ("GET", "/api/v1/credits") => (
                200,
                "application/json",
                r#"{"data":{"total_credits":12.5,"total_usage":2.25}}"#.into(),
            ),
            ("GET", "/api/v1/keys") => (200, "application/json", r#"{"data":[]}"#.into()),
            ("POST", "/api/v1/chat/completions") => (200, "text/event-stream", CHAT_SSE.into()),
            (_, p)
                if p.ends_with("/chat/completions")
                    || p.ends_with("/messages")
                    || p.ends_with("/responses") =>
            {
                (200, "application/json", r#"{"ok":true}"#.into())
            }
            _ => (
                404,
                "application/json",
                r#"{"error":{"message":"not found"}}"#.into(),
            ),
        }
    })
}

fn temp_home(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("anyr-e2e-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `anyr` with a clean environment: no key, no management key, no update
/// check, its own ANYROUTER_HOME.
fn anyr(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_anyr"));
    cmd.env("ANYR_NO_UPDATE", "1")
        .env("ANYR_NO_CATALOG", "1")
        .env("NO_COLOR", "1")
        .env("ANYROUTER_HOME", home)
        .env_remove("ANYROUTER_API_KEY")
        .env_remove("ANYROUTER_MANAGEMENT_KEY")
        .env_remove("ANYROUTER_PROFILE")
        .stdin(Stdio::null());
    cmd
}

fn run(cmd: &mut Command) -> (i32, String, String) {
    let out: Output = cmd.output().expect("spawn anyr");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn run_stdin(cmd: &mut Command, input: &str) -> (i32, String, String) {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn anyr");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}):\n{text}"))
}

/// Sign in through stdin, the way scripts and CI do.
fn login(home: &Path, base: &str) {
    let (code, out, err) = run_stdin(
        anyr(home).args(["login", "--key", "-", "--base-url", base]),
        &format!("{KEY}\n"),
    );
    assert_eq!(code, 0, "login failed:\n{out}{err}");
}

#[test]
fn every_command_and_subcommand_help_exits_zero_without_panicking() {
    // The list comes from the binary's own completion table, so a command
    // added later is covered without editing this test.
    let home = temp_home("help");
    let (code, out, err) = run(anyr(&home).args(["__complete", ""]));
    assert_eq!(code, 0, "{err}");
    let commands: Vec<String> = out
        .lines()
        .filter_map(|l| l.split('\t').next())
        .filter(|c| !c.is_empty() && !c.starts_with('-'))
        .map(String::from)
        .collect();
    assert!(
        commands.len() >= 30,
        "completion table looks broken: {commands:?}"
    );
    let mut checked = 0;
    for cmd in &commands {
        let (_, subs, _) = run(anyr(&home).args(["__complete", cmd, ""]));
        let mut targets = vec![vec![cmd.clone()]];
        for sub in subs.lines().filter_map(|l| l.split('\t').next()) {
            if !sub.is_empty() && !sub.starts_with('-') {
                targets.push(vec![cmd.clone(), sub.to_string()]);
            }
        }
        for t in targets {
            let (code, out, err) = run(anyr(&home).args(&t).arg("--help"));
            let all = format!("{out}{err}");
            assert_eq!(
                code,
                0,
                "`anyr {} --help` exited {code}:\n{all}",
                t.join(" ")
            );
            assert!(
                !out.trim().is_empty(),
                "`anyr {} --help` printed nothing",
                t.join(" ")
            );
            assert!(
                !all.contains("panicked"),
                "`anyr {} --help` panicked:\n{all}",
                t.join(" ")
            );
            checked += 1;
        }
    }
    assert!(checked > commands.len(), "no subcommands were checked");
}

#[test]
fn login_from_stdin_saves_the_key_privately_and_every_command_uses_it() {
    let gw = gateway();
    let home = temp_home("login");
    login(&home, &gw.base);

    // Login checks the key against the gateway with that key.
    let checks = gw.hits("/api/v1/");
    assert!(!checks.is_empty(), "login sent nothing");
    assert!(
        checks.iter().all(|r| r.auth == format!("Bearer {KEY}")),
        "{checks:?}"
    );

    let cfg = home.join("config.yaml");
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains(KEY), "key not saved:\n{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&cfg).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config with a key must be private");
    }

    // whoami --json: the saved key, redacted, and the saved base URL.
    let (code, out, err) = run(anyr(&home).args(["whoami", "--json"]));
    assert_eq!(code, 0, "{err}");
    let who = json(&out);
    assert_eq!(who["base_url"], gw.base.as_str());
    let shown = who["key_masked"].as_str().unwrap();
    assert!(shown.starts_with("sk-ar-v1-") && shown != KEY, "{shown}");
    assert!(
        !out.contains(KEY) && !err.contains(KEY),
        "whoami leaked the key"
    );

    // usage --json reads credits with the saved key.
    let (code, out, err) = run(anyr(&home).args(["usage", "--json"]));
    assert_eq!(code, 0, "{err}");
    assert!(json(&out).is_object(), "{out}");

    // logout removes the key from disk.
    let (code, _, err) = run(anyr(&home).args(["logout"]));
    assert_eq!(code, 0, "{err}");
    assert!(!std::fs::read_to_string(&cfg).unwrap().contains(KEY));
    let (code, _, _) = run(anyr(&home).args(["usage"]));
    assert_ne!(code, 0, "usage must fail after logout");
}

#[test]
fn chat_streams_the_answer_with_the_saved_key() {
    let gw = gateway();
    let home = temp_home("chat");
    login(&home, &gw.base);
    let (code, out, err) =
        run(anyr(&home).args(["chat", "say", "ping", "--model", "z-ai/glm-4.7"]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "pong\n");
    let sent = gw.hits("/api/v1/chat/completions");
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].auth, format!("Bearer {KEY}"));
    let body = json(&sent[0].body);
    assert_eq!(body["model"], "z-ai/glm-4.7");
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["messages"].as_array().unwrap().last().unwrap()["content"],
        "say ping"
    );
}

#[test]
fn models_json_is_the_catalog_shape_scripts_rely_on() {
    let gw = gateway();
    let home = temp_home("models");
    let (code, out, err) = run(anyr(&home).env_remove("ANYR_NO_CATALOG").args([
        "models",
        "--json",
        "--key",
        KEY,
        "--base-url",
        &gw.base,
    ]));
    assert_eq!(code, 0, "{err}");
    let v = json(&out);
    let ids: Vec<&str> = v["models"]
        .as_array()
        .unwrap_or_else(|| panic!("no models array:\n{out}"))
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"z-ai/glm-4.7"), "{ids:?}");

    let (code, out, err) = run(anyr(&home).env_remove("ANYR_NO_CATALOG").args([
        "models",
        "--free",
        "--json",
        "--key",
        KEY,
        "--base-url",
        &gw.base,
    ]));
    assert_eq!(code, 0, "{err}");
    let free: Vec<String> = json(&out)["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert!(free.contains(&"nvidia/free-a".to_string()), "{free:?}");
    assert!(
        !free.contains(&"z-ai/glm-4.7".to_string()),
        "paid model in --free: {free:?}"
    );
}

/// Launch `anyr <agent>` with tests/fake-agent.py standing in for the agent,
/// and return the requests it made to the gateway.
#[cfg(unix)]
fn launch_fake(agent: &str, version: &str, extra: &[&str]) -> (Mock, Vec<Req>) {
    let gw = gateway();
    let home = temp_home(&format!("launch-{agent}-{version}"));
    login(&home, &gw.base);
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake-agent.py");
    let path_var = format!("ANYROUTER_{}_PATH", agent.to_ascii_uppercase());
    let (code, out, err) = run(anyr(&home)
        .args([agent, "--yes", "--model", "z-ai/glm-4.7"])
        .args(extra)
        .env(&path_var, &fake)
        .env("FAKE_AGENT", agent)
        .env("FAKE_AGENT_VERSION", version)
        // A management key in the shell must never reach the agent.
        .env("ANYROUTER_MANAGEMENT_KEY", "ak_e2e_management_key")
        // Nor an unrelated provider key the user happens to export.
        .env("OPENAI_API_KEY", "sk-openai-not-anyrouter"));
    assert_eq!(
        code, 0,
        "anyr {agent} ({version}) failed:\nstdout:{out}\nstderr:{err}"
    );
    assert!(
        !out.contains(KEY) && !err.contains(KEY),
        "{agent} printed the key"
    );
    let inference: Vec<Req> = gw
        .seen()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path.starts_with("/api/v1/"))
        .collect();
    (gw, inference)
}

#[cfg(unix)]
fn assert_agent_used_the_key(agent: &str, version: &str, path: &str, reqs: &[Req]) {
    assert_eq!(reqs.len(), 1, "{agent} ({version}) requests: {reqs:?}");
    assert_eq!(
        reqs[0].path, path,
        "{agent} ({version}) hit the wrong route"
    );
    assert_eq!(
        reqs[0].auth,
        format!("Bearer {KEY}"),
        "{agent} ({version}) sent the wrong credential"
    );
}

#[cfg(unix)]
#[test]
fn pi_sends_the_inference_key_on_old_and_new_pi() {
    // Pi 0.73 reads a plain `apiKey` as an env var name; 0.79.4+ sends it as
    // the key itself, which made new Pi send "ANYROUTER_API_KEY" and get a
    // 401 (#173). Both must send the real key.
    for version in ["old", "new"] {
        let (_gw, reqs) = launch_fake("pi", version, &[]);
        assert_agent_used_the_key("pi", version, "/api/v1/chat/completions", &reqs);
        assert_eq!(json(&reqs[0].body)["model"], "z-ai/glm-4.7");
    }
}

#[cfg(unix)]
#[test]
fn claude_and_codex_send_the_inference_key_to_the_gateway() {
    let (_gw, reqs) = launch_fake("claude", "current", &[]);
    assert_agent_used_the_key("claude", "current", "/api/v1/messages", &reqs);
    assert_eq!(json(&reqs[0].body)["model"], "z-ai/glm-4.7");

    let (_gw, reqs) = launch_fake("codex", "current", &[]);
    assert_agent_used_the_key("codex", "current", "/api/v1/responses", &reqs);
    assert_eq!(json(&reqs[0].body)["model"], "z-ai/glm-4.7");
}

#[test]
fn artifact_rm_json_prints_a_result_object() {
    let gw = Mock::start(|r| match r.method.as_str() {
        "DELETE" => (200, "application/json", r#"{"ok":true}"#.into()),
        _ => (404, "application/json", "{}".into()),
    });
    let home = temp_home("artifact-rm");
    let (code, out, err) = run(anyr(&home).args([
        "artifact",
        "rm",
        "AbCdEf0123456789",
        "--yes",
        "--json",
        "--key",
        KEY,
        "--base-url",
        &gw.base,
    ]));
    assert_eq!(code, 0, "{err}");
    let v = json(&out);
    assert_eq!(v["id"], "AbCdEf0123456789");
    assert_eq!(v["deleted"], true);
    assert_eq!(
        gw.hits("/api/v1/artifacts/AbCdEf0123456789")[0].method,
        "DELETE"
    );
}

/// Stops the background relay even when an assertion fails.
#[cfg(unix)]
struct RelayGuard(PathBuf);

#[cfg(unix)]
impl Drop for RelayGuard {
    fn drop(&mut self) {
        let _ = anyr(&self.0).args(["relay", "stop"]).output();
    }
}

#[cfg(unix)]
#[test]
fn relay_start_detach_serves_a_cloud_request_then_stop_ends_it() {
    use std::collections::BTreeMap;
    use std::time::Instant;
    use tungstenite::Message;

    // Local model server the relay forwards to.
    let target = Mock::start(|r| match r.path.as_str() {
        "/models" | "/v1/models" => (
            200,
            "application/json",
            r#"{"data":[{"id":"stub-model"}]}"#.into(),
        ),
        _ => (
            200,
            "application/json",
            r#"{"choices":[{"message":{"role":"assistant","content":"hello from the stub"}}]}"#
                .into(),
        ),
    });
    let target_url = target.base.trim_end_matches("/api").to_string();

    // Mock cloud: takes the relay's socket, sends one request down, and
    // records what comes back.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let ws_url = format!(
        "ws://{}/api/v1/relay/connect",
        listener.local_addr().unwrap()
    );
    let result: Arc<Mutex<BTreeMap<String, String>>> = Arc::default();
    let result_in = Arc::clone(&result);
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let auth = Arc::new(Mutex::new(String::new()));
        let auth_in = Arc::clone(&auth);
        #[allow(clippy::result_large_err)]
        let on_handshake =
            move |req: &tungstenite::handshake::server::Request,
                  resp: tungstenite::handshake::server::Response| {
                *auth_in.lock().unwrap() = req
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                Ok(resp)
            };
        let mut ws = tungstenite::accept_hdr(stream, on_handshake).unwrap();
        let _ = ws.get_ref().set_read_timeout(Some(Duration::from_secs(20)));
        let note = |k: &str, v: String| {
            result_in.lock().unwrap().insert(k.into(), v);
        };
        note("auth", auth.lock().unwrap().clone());
        let Ok(hello) = ws.read() else { return };
        note("hello", hello.to_text().unwrap_or("").to_string());
        let body = r#"{"model":"stub-model","messages":[{"role":"user","content":"hi"}]}"#;
        let _ = ws.send(Message::text(
            serde_json::json!({"type":"request","id":"req-1","path":"/chat/completions","body":body,"stream":false})
                .to_string(),
        ));
        let mut text = String::new();
        while let Ok(msg) = ws.read() {
            let Ok(v) = serde_json::from_str::<Value>(msg.to_text().unwrap_or("")) else {
                continue;
            };
            if v["type"] == "chunk" {
                text.push_str(v["data"].as_str().unwrap_or(""));
            }
            if v["type"] == "done" || v["type"] == "error" {
                note("last", v["type"].as_str().unwrap_or("").to_string());
                break;
            }
        }
        note("text", text);
        // Hold the socket open until the relay is stopped.
        while ws.read().is_ok() {}
    });

    let home = temp_home("relay");
    let _guard = RelayGuard(home.clone());
    let (code, out, err) = run(anyr(&home).args([
        "relay",
        "start",
        "--detach",
        "--no-pool",
        "--url",
        &ws_url,
        "--target",
        &target_url,
        "--key",
        KEY,
    ]));
    assert_eq!(code, 0, "relay start --detach failed:\n{out}{err}");

    let deadline = Instant::now() + Duration::from_secs(30);
    while !result.lock().unwrap().contains_key("text") {
        assert!(
            Instant::now() < deadline,
            "no answer through the relay: {:?}",
            result.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let r = result.lock().unwrap().clone();
    assert_eq!(
        r["auth"],
        format!("Bearer {KEY}"),
        "relay must authenticate with the key"
    );
    assert!(
        r["hello"].contains("stub-model"),
        "hello did not list the model: {}",
        r["hello"]
    );
    assert_eq!(r["last"], "done");
    assert!(r["text"].contains("hello from the stub"), "{}", r["text"]);

    let (code, out, err) = run(anyr(&home).args(["relay", "status", "--json"]));
    assert_eq!(code, 0, "{err}");
    let status = json(&out);
    assert_eq!(status["running"], true, "{out}");
    assert!(status["pid"].as_u64().is_some(), "{out}");

    let (code, out, err) = run(anyr(&home).args(["relay", "stop"]));
    assert_eq!(code, 0, "{out}{err}");
    let (_, out, _) = run(anyr(&home).args(["relay", "status", "--json"]));
    assert_eq!(
        json(&out)["running"],
        false,
        "relay still running after stop:\n{out}"
    );
}

/// Runs a foreground relay whose config cannot be rewritten against a cloud
/// that names a new device on each of the first two sockets, and returns the
/// handshake URIs of all three connections.
#[cfg(unix)]
fn relay_handshakes_with_unwritable_config(seed: Option<&str>) -> Vec<String> {
    use std::time::Instant;
    use tungstenite::Message;

    let target = Mock::start(|_| {
        (
            200,
            "application/json",
            r#"{"data":[{"id":"stub-model"}]}"#.into(),
        )
    });
    let target_url = target.base.trim_end_matches("/api").to_string();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let ws_url = format!(
        "ws://{}/api/v1/relay/connect",
        listener.local_addr().unwrap()
    );
    let uris: Arc<Mutex<Vec<String>>> = Arc::default();
    let uris_in = Arc::clone(&uris);
    std::thread::spawn(move || {
        for announce in [Some("dev-1"), Some("dev-2"), None] {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let uris_h = Arc::clone(&uris_in);
            #[allow(clippy::result_large_err)]
            let on_handshake =
                move |req: &tungstenite::handshake::server::Request,
                      resp: tungstenite::handshake::server::Response| {
                    uris_h.lock().unwrap().push(req.uri().to_string());
                    Ok(resp)
                };
            let Ok(mut ws) = tungstenite::accept_hdr(stream, on_handshake) else {
                return;
            };
            let _ = ws.get_ref().set_read_timeout(Some(Duration::from_secs(20)));
            if ws.read().is_err() {
                return;
            }
            if let Some(id) = announce {
                let _ = ws.send(Message::text(
                    serde_json::json!({"type":"device","id":id,"name":"Desk"}).to_string(),
                ));
                std::thread::sleep(Duration::from_millis(300));
                let _ = ws.close(None);
                let _ = ws.flush();
            } else {
                while ws.read().is_ok() {}
            }
        }
    });

    let home = temp_home("relay-unwritable");
    if let Some(id) = seed {
        std::fs::write(
            home.join("config.yaml"),
            format!("active_profile: default\nprofiles:\n  default:\n    relay_device_id: {id}\n"),
        )
        .unwrap();
    }
    // A non-empty directory squats on the temp path the config writer uses,
    // so every attempt to save the device id fails.
    let squat = home.join("config.yaml.tmp");
    std::fs::create_dir_all(&squat).unwrap();
    std::fs::write(squat.join("keep"), "").unwrap();

    let mut child = anyr(&home)
        .args([
            "relay",
            "start",
            "--no-pool",
            "--no-launch",
            "--url",
            &ws_url,
            "--target",
            &target_url,
            "--key",
            KEY,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn relay");
    let deadline = Instant::now() + Duration::from_secs(60);
    while uris.lock().unwrap().len() < 3 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    let seen = uris.lock().unwrap().clone();
    assert_eq!(seen.len(), 3, "relay did not reconnect twice: {seen:?}");
    seen
}

/// WHY: the device id from the cloud's `device` frame is only cached in the
/// config file. When that write fails, every reconnect used to send
/// `fresh=1` and the cloud minted a new device row each time until the
/// 5-device cap blocked the relay.
#[cfg(unix)]
#[test]
fn relay_keeps_the_device_id_in_memory_when_the_config_is_unwritable() {
    let seen = relay_handshakes_with_unwritable_config(None);
    assert!(seen[0].contains("fresh=1"), "{}", seen[0]);
    assert!(seen[1].contains("device=dev-1"), "{}", seen[1]);
    assert!(!seen[1].contains("fresh=1"), "{}", seen[1]);
}

/// WHY: a stale cached id must give way to the one the cloud just named even
/// when the cache cannot be overwritten.
#[cfg(unix)]
#[test]
fn relay_prefers_the_new_device_id_over_a_stale_cached_one() {
    let seen = relay_handshakes_with_unwritable_config(Some("stale-0"));
    assert!(seen[0].contains("device=stale-0"), "{}", seen[0]);
    assert!(seen[1].contains("device=dev-1"), "{}", seen[1]);
    assert!(seen[2].contains("device=dev-2"), "{}", seen[2]);
}
