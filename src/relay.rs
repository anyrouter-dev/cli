//! `anyr relay` — pair this machine with an AnyRouter account and proxy cloud
//! chat-completion requests to a local OpenAI-compatible server. Migrated from
//! packages/agent/src/commands/relay.ts (TS CLI #1099); the cloud side
//! (`MacBridgeDO`) is live, so only this client was missing.
//!
//! Zero-setup auth: `anyr relay start` resolves credentials in this order —
//!   1. explicit `--token`
//!   2. `ANYROUTER_RELAY_TOKEN` env var
//!   3. `relay_token` in the active profile of ~/.anyrouter/config.yaml
//!   4. AUTO-PAIR with an existing sk-ar-v1 inference key (--token-less chain:
//!      flag/env/config) → POST /api/v1/relay/devices, store the minted rk_
//!   5. DEVICE LOGIN when no credentials exist at all — RFC 8628 flow mints an
//!      sk-ar key (stored), then auto-pair as in 4. An sk-ar key alone is
//!      always sufficient; device login is a convenience path, not a
//!      requirement.
//!
//! Both paths store the rk_ token under `relay_token` in the shared
//! ~/.anyrouter/config.yaml, so the TS CLI picks it up too.
//!
//! Transport: one outbound WebSocket to the cloud relay (via
//! `/api/v1/relay/connect`), auto-reconnecting with exponential backoff. For
//! each `request` frame pushed down, the device answers with `head`/`chunk`/
//! `done` frames. On macOS, when the system model is available, those frames
//! come from the in-process helper and the hello frame advertises
//! `foundation-model`. Otherwise the body is forwarded to a local
//! OpenAI-compatible server. `fm serve` on :1976 also advertises
//! `foundation-model` — that is the pool join key, not the id its own
//! `/v1/models` returns. :8000 and Ollama advertise only their own list.
//! `cancel` frames abort the in-flight request.
//!
//! Process model: the socket lives on the main thread (tungstenite is not
//! Sync, so readers and writers share one owner). Each incoming `request`
//! frame spawns a worker thread that streams the local response into an
//! outbound channel; the main loop multiplexes between draining that channel
//! and polling the socket (short read timeout → `WouldBlock`). This keeps
//! concurrent requests streaming without deadlocking the single socket.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(feature = "native")]
use serde_json::json;

/// Profile field storing the rk_ pairing token (~/.anyrouter/config.yaml).
pub const RELAY_TOKEN_FIELD: &str = "relay_token";
pub const RELAY_TOKEN_ENV_VAR: &str = "ANYROUTER_RELAY_TOKEN";
/// Cached paired-device id so `--pool` doesn't need another lookup.
pub const RELAY_DEVICE_ID_FIELD: &str = "relay_device_id";

const DEFAULT_TARGET: &str = "http://localhost:8000/v1";
const DEFAULT_WS_URL: &str = "wss://anyrouter.dev/api/v1/relay/connect";
const DEFAULT_API_BASE: &str = "https://anyrouter.dev/api/v1";
const DEFAULT_DEVICE_NAME: &str = "My Mac";
const MAX_BACKOFF_MS: u64 = 30_000;
const BASE_BACKOFF_MS: u64 = 1_000;
/// How long one non-blocking poll of the socket waits before we check the
/// outbound queue again. Bounds cancel latency and reconnect responsiveness.
const SOCKET_POLL_MS: u64 = 250;

/// `fm serve`'s loopback OpenAI-compatible endpoint: GET /health and /v1/models.
const FM_SERVE_TARGET: &str = "http://127.0.0.1:1976/v1";
const FM_SERVE_HEALTH_URL: &str = "http://127.0.0.1:1976/health";
const OLLAMA_TARGET: &str = "http://localhost:11434/v1";
/// Model id `fm serve` accepts for the on-device system model. The cloud
/// sends the pool join key `foundation-model` instead.
const FM_SERVE_LOCAL_MODEL_ID: &str = "system";

// ---------------------------------------------------------------------------
// Diagnostics — verbose-gated [relay] lines go to stderr like the TS client.
// ---------------------------------------------------------------------------

static VERBOSE: AtomicBool = AtomicBool::new(false);

fn vlog(msg: &str) {
    if VERBOSE.load(Ordering::Relaxed) {
        eprintln!("[relay] {msg}");
    }
}

fn ulog(msg: &str) {
    eprintln!("{msg}");
}

// ---------------------------------------------------------------------------
// Wire protocol — mirror of packages/contracts/src/local-relay/index.ts.
// Frames are JSON text messages sharing a `{ type, id }` envelope. Keep in
// sync with that file; unknown types are ignored gracefully so old/new builds
// stay forward/backward compatible.
// ---------------------------------------------------------------------------

/// Cloud → device: execute a chat completion against the local target.
struct RequestFrame {
    id: String,
    path: String,
    body: String,
}

/// Device → cloud frames.
enum ClientFrame {
    Head {
        id: String,
        status: u16,
        content_type: String,
    },
    Chunk {
        id: String,
        data: String,
    },
    Done {
        id: String,
    },
    Error {
        id: String,
        message: String,
    },
    /// Capability advertisement (#1128), sent right after connecting (and on
    /// every reconnect): OpenAI-compatible model ids this device can serve.
    /// Pool routing joins requests against these ids.
    Hello {
        models: Vec<String>,
        max_concurrency: Option<u32>,
    },
}

impl ClientFrame {
    fn to_json(&self) -> String {
        match self {
            Self::Head {
                id,
                status,
                content_type,
            } => json!({
                "type": "head", "id": id, "status": status, "contentType": content_type,
            })
            .to_string(),
            Self::Chunk { id, data } => {
                json!({ "type": "chunk", "id": id, "data": data }).to_string()
            }
            Self::Done { id } => json!({ "type": "done", "id": id }).to_string(),
            Self::Error { id, message } => {
                json!({ "type": "error", "id": id, "message": message }).to_string()
            }
            Self::Hello {
                models,
                max_concurrency,
            } => {
                let mut v = json!({ "type": "hello", "id": "", "models": models });
                if let Some(n) = max_concurrency {
                    v["maxConcurrency"] = json!(n);
                }
                v.to_string()
            }
        }
    }
}

/// Parse a raw WS text message into a server frame, mirroring
/// parseRelayServerFrame() in packages/contracts/src/local-relay/index.ts:
/// malformed JSON, missing `{type,id}`, or unknown types yield None (ignored
/// rather than fatal). `Some(Ok(..))` is a request; `Some(Err(id))` a cancel.
fn parse_server_frame(raw: &str) -> Option<Result<RequestFrame, String>> {
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = parsed.as_object()?;
    let ty = obj.get("type")?.as_str()?;
    let id = obj.get("id")?.as_str()?;
    match ty {
        "request" => Some(Ok(RequestFrame {
            id: id.to_string(),
            path: obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            body: obj
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })),
        "cancel" => Some(Err(id.to_string())),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

struct StartArgs {
    /// None = not explicitly passed; run_relay_start auto-detects then.
    target: Option<String>,
    token: Option<String>,
    url: Option<String>,
    name: String,
    pool: bool,
    max_concurrency: Option<u32>,
}

fn parse_start_args(parsed: &crate::parse::ParsedArgs) -> StartArgs {
    let get = |name: &str| crate::parse::get_string_flag(&parsed.flags, name);
    StartArgs {
        target: get("target"),
        token: get("token"),
        url: get("url"),
        name: get("name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_DEVICE_NAME.into()),
        pool: parsed.flag_true("pool"),
        max_concurrency: get("max-concurrency")
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|n| *n > 0),
    }
}

// ---------------------------------------------------------------------------
// Pairing + credential resolution
// ---------------------------------------------------------------------------

/// Mint an rk_ pairing token via POST /relay/devices with the user's sk-ar-v1
/// inference key (the route accepts inference keys) and persist it to the
/// shared config. Shared by explicit `relay pair` and auto-pair in start.
fn pair_device(
    api_key: &str,
    name: &str,
    env: &BTreeMap<String, String>,
) -> Result<String, String> {
    let url = format!("{DEFAULT_API_BASE}/relay/devices");
    let body = json!({ "name": name }).to_string();
    let (status, resp) = crate::http::http_post(&url, Some(api_key), Some(&body))?;
    let value: serde_json::Value = serde_json::from_str(&resp).unwrap_or(serde_json::Value::Null);
    let token = value.get("token").and_then(|v| v.as_str()).unwrap_or("");
    if !(200..300).contains(&status) || token.is_empty() {
        let msg = value
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or("unexpected response");
        return Err(format!("Pairing failed ({status}): {msg}"));
    }
    write_profile_field(RELAY_TOKEN_FIELD, token, env)?;
    // Cached so --pool can PATCH /relay/devices/:id without another lookup.
    if let Some(id) = value
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        write_profile_field(RELAY_DEVICE_ID_FIELD, id, env)?;
    }
    Ok(token.to_string())
}

/// Resolve the credential chain documented in the module comment. Returns the
/// rk_ relay token ready for the connect handshake.
fn ensure_relay_token(
    explicit: Option<&str>,
    device_name: &str,
    parsed: &crate::parse::ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<String, String> {
    if let Some(t) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(t.to_string());
    }
    if let Some(t) = env
        .get(RELAY_TOKEN_ENV_VAR)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return Ok(t.to_string());
    }
    if let Some(t) = read_stored_relay_token(env)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Ok(t);
    }
    let path = crate::config::resolve_config_path(None, env);

    // No relay token yet: resolve (or mint) an sk-ar inference key, auto-pair.
    let api_key = match crate::key::resolve_api_key(
        &parsed.flags,
        env,
        crate::key::load_config_if_present(&path)
            .as_ref()
            .and_then(|c| c.profiles.get(&c.active_profile)),
    ) {
        Some(k) => k,
        None => {
            // Interactive convenience path only: a headless invocation
            // (cron, CI, piped output) must not sit polling a device code.
            if !crate::term::is_interactive() {
                let bin = crate::help::invoked_bin();
                return Err(format!(
                    "No relay token and no AnyRouter key. Run `{bin} login`, \
or set ANYROUTER_API_KEY / {RELAY_TOKEN_ENV_VAR}, or pass --token."
                ));
            }
            ulog("No AnyRouter credentials found — starting device login.");
            let open_browser = crate::auth::browser_likely_available(env);
            let token =
                crate::auth::login_via_device(&device_login_base(), Some("cli"), open_browser)?;
            persist_api_key(&token.api_key, env)?;
            ulog("Logged in.");
            token.api_key
        }
    };

    ulog("No relay token found — pairing this device automatically…");
    let token = pair_device(&api_key, device_name, env)?;
    ulog(&format!(
        "Paired as \"{device_name}\". Token saved to {}.",
        path.display()
    ));
    Ok(token)
}

fn read_stored_relay_token(env: &BTreeMap<String, String>) -> Option<String> {
    let path = crate::config::resolve_config_path(None, env);
    crate::key::load_config_if_present(&path)?
        .profiles
        .get(&load_active_profile_name(env))?
        .relay_token
        .clone()
}

fn load_active_profile_name(env: &BTreeMap<String, String>) -> String {
    let path = crate::config::resolve_config_path(None, env);
    crate::key::load_config_if_present(&path)
        .map(|c| c.active_profile)
        .unwrap_or_else(|| crate::config::DEFAULT_PROFILE.into())
}

/// `…/api/v1` → `…/api` — the parent base the existing device-flow helpers
/// expect (they append /v1/auth/... themselves).
fn device_login_base() -> String {
    DEFAULT_API_BASE
        .strip_suffix("/v1")
        .unwrap_or(DEFAULT_API_BASE)
        .to_string()
}

/// Write one relay-related field (or the api_key after device login) into the
/// active profile of the shared config.
fn write_profile_field(
    field: &str,
    value: &str,
    env: &BTreeMap<String, String>,
) -> Result<(), String> {
    let path = crate::config::resolve_config_path(None, env);
    let mut cfg = crate::key::load_config_if_present(&path).unwrap_or_default();
    let active = cfg.active_profile.clone();
    let entry = cfg.profiles.entry(active).or_default();
    match field {
        RELAY_TOKEN_FIELD => entry.relay_token = Some(value.to_string()),
        RELAY_DEVICE_ID_FIELD => entry.relay_device_id = Some(value.to_string()),
        "api_key" => entry.api_key = Some(value.to_string()),
        other => return Err(format!("unknown relay profile field {other}")),
    }
    crate::config::write_config(&cfg, &path)
}

fn persist_api_key(key: &str, env: &BTreeMap<String, String>) -> Result<(), String> {
    write_profile_field("api_key", key, env)
}

// ---------------------------------------------------------------------------
// Target detection
// ---------------------------------------------------------------------------

struct Probe {
    target: &'static str,
    label: &'static str,
    health_url: &'static str,
}

/// HTTP probe order after the in-process system model is unavailable.
const RELAY_PROBES: &[Probe] = &[
    Probe {
        target: FM_SERVE_TARGET,
        label: "fm serve",
        health_url: FM_SERVE_HEALTH_URL,
    },
    Probe {
        target: DEFAULT_TARGET,
        label: "default target",
        health_url: "http://localhost:8000/v1/models",
    },
    Probe {
        target: OLLAMA_TARGET,
        label: "Ollama",
        health_url: "http://localhost:11434/v1/models",
    },
];

/// Auto-detect a local OpenAI-compatible server when `--target` isn't given:
/// probe fm serve (:1976), then the historical default (:8000), then Ollama
/// (:11434) — the first one that responds AT ALL wins (any HTTP response
/// counts, not just 2xx; the point is only "something is listening").
/// Falls back to the historical default with a clear log line otherwise.
fn detect_relay_target() -> &'static str {
    for p in RELAY_PROBES {
        if http_probe_listening(p.health_url) {
            ulog(&format!(
                "auto-detected local server: {} at {}",
                p.label, p.target
            ));
            return p.target;
        }
    }
    ulog(&format!(
        "no local server auto-detected — falling back to {DEFAULT_TARGET}. \
         Start `fm serve` (or Ollama / LM Studio), or pass --target explicitly."
    ));
    DEFAULT_TARGET
}

fn resolve_backend(explicit: Option<&str>) -> RelayBackend {
    let explicit = explicit.map(str::trim).filter(|s| !s.is_empty());
    let fm_available = explicit.is_none() && crate::foundation::foundation_model_available();
    if fm_available {
        ulog("serving foundation-model");
    }
    let detected = if explicit.is_none() && !fm_available {
        detect_relay_target()
    } else {
        ""
    };
    choose_backend(explicit, fm_available, detected)
}

fn detect_help_line() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "auto-detects the system model when it is available, else fm serve on :1976, then :8000, then Ollama on :11434"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "auto-detects fm serve on :1976, then :8000, then Ollama on :11434"
    }
}

/// True when something answers within PROBE_TIMEOUT_MS (any status counts).
fn http_probe_listening(url: &str) -> bool {
    matches!(
        ureq::get(url).timeout(Duration::from_millis(800)).call(),
        Ok(_) | Err(ureq::Error::Status(_, _))
    )
}

// ---------------------------------------------------------------------------
// Capability advertisement (hello frame)
// ---------------------------------------------------------------------------

/// Fetch the local target's OpenAI-compatible /models list. Empty vec when
/// unreachable — connect without advertising rather than failing: a device
/// with no advertised models simply never becomes a pool donor.
fn fetch_local_models(target: &str) -> Vec<String> {
    let url = format!("{}/models", target.trim_end_matches('/'));
    let result = ureq::get(&url).timeout(Duration::from_secs(5)).call();
    match result {
        Ok(resp) if (200..300).contains(&resp.status()) => {
            let body = resp.into_string().unwrap_or_default();
            serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("data").and_then(|d| d.as_array()).map(|arr| {
                        arr.iter()
                            .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
                            .map(str::to_string)
                            .collect()
                    })
                })
                .unwrap_or_default()
        }
        Ok(resp) => {
            vlog(&format!(
                "could not reach {url} (status {}) — connecting without capability advertisement",
                resp.status()
            ));
            Vec::new()
        }
        Err(err) => {
            vlog(&format!(
                "could not reach {url} ({err}) — connecting without capability advertisement"
            ));
            Vec::new()
        }
    }
}

/// Where one `anyr relay` process answers request frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RelayBackend {
    /// System language model, in-process. Hello id is `foundation-model`.
    Foundation,
    /// OpenAI-compatible HTTP server. Hello ids are that server's `/v1/models`.
    Http(&'static str),
}

/// Pick the backend for this process. An explicit `--target` is that HTTP
/// server. With no target, an available system model wins. Otherwise `detected`
/// is the first HTTP probe that answered.
fn choose_backend(
    explicit_target: Option<&str>,
    fm_available: bool,
    detected: &'static str,
) -> RelayBackend {
    if let Some(target) = explicit_target.map(str::trim).filter(|s| !s.is_empty()) {
        return RelayBackend::Http(Box::leak(target.to_string().into_boxed_str()));
    }
    if fm_available {
        RelayBackend::Foundation
    } else {
        RelayBackend::Http(detected)
    }
}

/// True when `target` is `fm serve`'s loopback endpoint.
fn is_fm_serve_target(target: &str) -> bool {
    target.contains("127.0.0.1:1976") || target.contains("localhost:1976")
}

/// Hello-frame model ids. The in-process system model and `fm serve` advertise
/// `foundation-model`, the upstream name pool routing joins on. The catalog id
/// `apple/foundation-model` is not that key. :8000 and Ollama advertise only
/// the ids they returned.
fn models_for_hello(backend: RelayBackend, fetched: &[String]) -> Vec<String> {
    match backend {
        RelayBackend::Foundation => vec![crate::foundation::FOUNDATION_MODEL_ID.to_string()],
        RelayBackend::Http(target) => {
            let mut models = fetched.to_vec();
            if is_fm_serve_target(target)
                && !models
                    .iter()
                    .any(|m| m == crate::foundation::FOUNDATION_MODEL_ID)
            {
                models.push(crate::foundation::FOUNDATION_MODEL_ID.to_string());
            }
            models
        }
    }
}

/// Body forwarded to an HTTP target. `fm serve` rejects the pool join key, so
/// `foundation-model` and the catalog id become its local id `system`. Other
/// servers, and any other model name, are forwarded unchanged.
fn body_for_http_target(target: &str, body: &str) -> String {
    if !is_fm_serve_target(target) {
        return body.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body) else {
        return body.to_string();
    };
    let Some(model) = value.get("model").and_then(|m| m.as_str()) else {
        return body.to_string();
    };
    if model == crate::foundation::FOUNDATION_MODEL_ID || model == "apple/foundation-model" {
        value["model"] = serde_json::json!(FM_SERVE_LOCAL_MODEL_ID);
        value.to_string()
    } else {
        body.to_string()
    }
}

// ---------------------------------------------------------------------------
// Local request handling (worker threads)
// ---------------------------------------------------------------------------

/// Forward one request frame to the local target and stream the response back
/// as head/chunk/done frames through `tx`. Chunks are enqueued as soon as they
/// are read from the local response — never buffered whole, so SSE (or any
/// streamed body) round-trips incrementally. Runs on its own thread; `cancel`
/// flips the flag and the stream loop bails between reads.
fn handle_request(
    tx: &mpsc::Sender<ClientFrame>,
    frame: &RequestFrame,
    target: &str,
    cancel: &AtomicBool,
) {
    let path = if frame.path.starts_with('/') {
        frame.path.clone()
    } else {
        format!("/{}", frame.path)
    };
    let url = format!("{}{}", target.trim_end_matches('/'), path);

    // Connect fast, tolerate slow generation: no overall cap, generous idle
    // gap between body reads.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(300))
        .build();

    let body = body_for_http_target(target, &frame.body);
    let resp = match agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body)
    {
        Ok(resp) => resp,
        Err(ureq::Error::Status(code, resp)) => {
            // Non-2xx from the local target still relays real status/body.
            let _ = tx.send(ClientFrame::Head {
                id: frame.id.clone(),
                status: code,
                content_type: content_type_of(&resp),
            });
            let body = resp.into_string().unwrap_or_default();
            if !body.is_empty() {
                let _ = tx.send(ClientFrame::Chunk {
                    id: frame.id.clone(),
                    data: body,
                });
            }
            let _ = tx.send(ClientFrame::Done {
                id: frame.id.clone(),
            });
            return;
        }
        Err(_) if cancel.load(Ordering::SeqCst) => return, // cancelled — no frame
        Err(err) => {
            let message = err.to_string();
            vlog(&format!("request {} failed: {message}", frame.id));
            let _ = tx.send(ClientFrame::Error {
                id: frame.id.clone(),
                message: format!("Local target unreachable: {message}"),
            });
            return;
        }
    };

    let _ = tx.send(ClientFrame::Head {
        id: frame.id.clone(),
        status: resp.status(),
        content_type: content_type_of(&resp),
    });

    // Stream the body in chunks. UTF-8 safe: any partial multi-byte sequence
    // stays buffered until its tail arrives instead of being split mid-codepoint.
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 8192];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        if cancel.load(Ordering::SeqCst) {
            return; // cancelled by cloud — stop relaying
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                let flush_len = utf8_flush_len(&pending);
                if flush_len > 0 {
                    let data = String::from_utf8_lossy(&pending[..flush_len]).into_owned();
                    pending.drain(..flush_len);
                    if tx
                        .send(ClientFrame::Chunk {
                            id: frame.id.clone(),
                            data,
                        })
                        .is_err()
                    {
                        return; // socket died — nothing more to do
                    }
                }
            }
            Err(err) => {
                if !cancel.load(Ordering::SeqCst) {
                    let message = err.to_string();
                    vlog(&format!(
                        "request {} failed mid-stream: {message}",
                        frame.id
                    ));
                    let _ = tx.send(ClientFrame::Error {
                        id: frame.id.clone(),
                        message: format!("Local target unreachable: {message}"),
                    });
                }
                return;
            }
        }
    }
    if !pending.is_empty() && !cancel.load(Ordering::SeqCst) {
        let data = String::from_utf8_lossy(&pending).into_owned();
        let _ = tx.send(ClientFrame::Chunk {
            id: frame.id.clone(),
            data,
        });
    }
    let _ = tx.send(ClientFrame::Done {
        id: frame.id.clone(),
    });
}

fn handle_foundation_request(
    tx: &mpsc::Sender<ClientFrame>,
    frame: &RequestFrame,
    cancel: &AtomicBool,
) {
    if !frame.path.is_empty() && !frame.path.contains("chat/completions") {
        let _ = tx.send(ClientFrame::Error {
            id: frame.id.clone(),
            message: format!("local relay does not serve {}", frame.path),
        });
        return;
    }
    let mut session = match crate::foundation::FmSession::from_chat_body(&frame.id, &frame.body) {
        Ok(session) => session,
        Err(message) => {
            let _ = tx.send(ClientFrame::Error {
                id: frame.id.clone(),
                message,
            });
            return;
        }
    };
    let tx = tx.clone();
    let id = frame.id.clone();
    crate::foundation::run_foundation_model(&mut session, cancel, move |event| {
        if matches!(&event, crate::foundation::FmEvent::Error(_)) {
            vlog(&format!("request {id} failed"));
        }
        let _ = tx.send(foundation_client_frame(&id, event));
    });
}

fn foundation_client_frame(id: &str, event: crate::foundation::FmEvent) -> ClientFrame {
    match event {
        crate::foundation::FmEvent::Head {
            status,
            content_type,
        } => ClientFrame::Head {
            id: id.to_string(),
            status,
            content_type,
        },
        crate::foundation::FmEvent::Chunk(data) => ClientFrame::Chunk {
            id: id.to_string(),
            data,
        },
        crate::foundation::FmEvent::Done => ClientFrame::Done { id: id.to_string() },
        crate::foundation::FmEvent::Error(message) => ClientFrame::Error {
            id: id.to_string(),
            message,
        },
    }
}

/// Longest prefix of `bytes` ending on a UTF-8 codepoint boundary.
fn utf8_flush_len(bytes: &[u8]) -> usize {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(e) => e.valid_up_to(),
    }
}

fn content_type_of(resp: &ureq::Response) -> String {
    resp.header("content-type")
        .unwrap_or("application/json")
        .to_string()
}

// ---------------------------------------------------------------------------
// Pool sharing (--pool)
// ---------------------------------------------------------------------------

/// Resolve this device's id for `--pool`: cached `relay_device_id` first; a
/// pre-cache pairing falls back to GET /relay/devices, using the result only
/// when exactly one active device exists — ambiguous otherwise, so point the
/// user at the dashboard instead of guessing.
fn resolve_relay_device_id(
    parsed: &crate::parse::ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<String, String> {
    let path = crate::config::resolve_config_path(None, env);
    if let Some(cfg) = crate::key::load_config_if_present(&path) {
        if let Some(p) = cfg.profiles.get(&cfg.active_profile) {
            if let Some(id) = p
                .relay_device_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Ok(id.to_string());
            }
        }
    }
    let api_key = resolve_sk_ar_key(parsed, env).ok_or_else(|| {
        "Could not resolve this device's id for --pool (no key found). Run: anyr relay pair."
            .to_string()
    })?;
    let url = format!("{DEFAULT_API_BASE}/relay/devices");
    let (status, body) = crate::http::http_get(&url, Some(&api_key))?;
    let value: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    let active_ids: Vec<&str> = value
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|d| d.get("status").and_then(|s| s.as_str()) == Some("active"))
                .filter_map(|d| d.get("id").and_then(|i| i.as_str()))
                .collect()
        })
        .unwrap_or_default();
    if !(200..300).contains(&status) || active_ids.is_empty() {
        return Err("No paired device found for --pool. Run: anyr relay pair".into());
    }
    if active_ids.len() > 1 {
        return Err(
            "Multiple paired devices found — can't tell which one to enable pool sharing for. \
             Manage pool sharing from the dashboard instead: https://dash.anyrouter.dev/devices"
                .into(),
        );
    }
    let id = active_ids[0].to_string();
    write_profile_field(RELAY_DEVICE_ID_FIELD, &id, env)?;
    Ok(id)
}

/// PATCH /relay/devices/:id { pool_enabled: true } with the resolved sk-ar
/// key. Opting in lets this device serve OTHER users' requests when its own
/// capacity is idle, earning credits per served request.
fn enable_relay_pool(
    device_id: &str,
    parsed: &crate::parse::ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<(), String> {
    let api_key = resolve_sk_ar_key(parsed, env).ok_or_else(|| {
        "Could not resolve an AnyRouter API key to enable pool sharing.".to_string()
    })?;
    let url = format!("{DEFAULT_API_BASE}/relay/devices/{device_id}");
    let body = json!({ "pool_enabled": true }).to_string();
    let (status, resp) = crate::http::http_patch(&url, Some(&api_key), Some(&body))?;
    if !(200..300).contains(&status) {
        let value: serde_json::Value =
            serde_json::from_str(&resp).unwrap_or(serde_json::Value::Null);
        let msg = value
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or("unexpected response");
        return Err(format!("Could not enable pool sharing ({status}): {msg}"));
    }
    Ok(())
}

fn resolve_sk_ar_key(
    parsed: &crate::parse::ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Option<String> {
    let path = crate::config::resolve_config_path(None, env);
    crate::key::resolve_api_key(
        &parsed.flags,
        env,
        crate::key::load_config_if_present(&path)
            .as_ref()
            .and_then(|c| c.profiles.get(&c.active_profile)),
    )
}

// ---------------------------------------------------------------------------
// Transport — outbound WebSocket with reconnect + exponential backoff
// ---------------------------------------------------------------------------

type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

fn connect_ws(url: &str, token: &str) -> Result<Ws, String> {
    use tungstenite::client::IntoClientRequest;
    use tungstenite::http::header::AUTHORIZATION;
    use tungstenite::http::HeaderValue;

    let mut request = url
        .into_client_request()
        .map_err(|e| format!("invalid relay url \"{url}\": {e}"))?;
    // sk-ar keys are accepted here too (one key for everything); never sent
    // via ?token= query — URLs land in access logs and must not carry keys.
    let value = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "relay token contains invalid characters".to_string())?;
    request.headers_mut().insert(AUTHORIZATION, value);
    // `tungstenite::client::connect` dials with no timeout, so a blackholed
    // relay host would hang the reconnect loop. Dial ourselves (15s, same as
    // the relay HTTP client), then let tungstenite do TLS + handshake.
    let uri = request.uri();
    let tls = uri.scheme_str() == Some("wss");
    let host = uri
        .host()
        .ok_or_else(|| format!("relay url \"{url}\" has no host"))?
        .to_string();
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
    let stream = dial_with_timeout(&host, port, WS_CONNECT_TIMEOUT)?;
    let (ws, _resp) = tungstenite::client_tls(request, stream).map_err(|e| e.to_string())?;
    Ok(ws)
}

const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

fn dial_with_timeout(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<std::net::TcpStream, String> {
    use std::net::ToSocketAddrs;
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("could not resolve relay host {host}: {e}"))?;
    let mut last = format!("relay host {host} resolved to no addresses");
    for addr in addrs {
        match std::net::TcpStream::connect_timeout(&addr, timeout) {
            Ok(s) => return Ok(s),
            Err(e) => last = format!("could not connect to relay {addr}: {e}"),
        }
    }
    Err(last)
}

/// Shared state one live connection owns.
struct ConnState {
    /// Outbound frames queued by worker threads; drained by the main loop.
    tx: mpsc::Sender<ClientFrame>,
    rx: Receiver<ClientFrame>,
    /// Cancel flags keyed by request id; `cancel` frames flip them.
    in_flight: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
}

/// One full connection lifecycle: handshake → hello → serve frames until the
/// socket dies. Returns when disconnected (caller reconnects with backoff).
fn serve_connection(
    ws: &mut Ws,
    backend: RelayBackend,
    state: &ConnState,
    max_concurrency: Option<u32>,
) {
    vlog("connected");

    // Capability advertisement on every connect/reconnect (#1128), so models
    // added to the local server show up at the next reconnect at latest.
    let fetched = match backend {
        RelayBackend::Foundation => Vec::new(),
        RelayBackend::Http(target) => fetch_local_models(target),
    };
    let models = models_for_hello(backend, &fetched);
    send_frame(
        ws,
        &ClientFrame::Hello {
            models: models.clone(),
            max_concurrency,
        },
    );
    ulog(&format!(
        "advertised {} model(s): {}",
        models.len(),
        if models.is_empty() {
            "(none)".into()
        } else {
            models.join(", ")
        }
    ));

    loop {
        // 1. Drain worker output first so streamed chunks go out promptly.
        loop {
            match state.rx.try_recv() {
                Ok(frame) => {
                    match &frame {
                        ClientFrame::Done { id } | ClientFrame::Error { id, .. } => {
                            lock_in_flight(&state.in_flight).remove(id);
                        }
                        _ => {}
                    }
                    send_frame(ws, &frame);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return, // unreachable; workers outlive rx
            }
        }

        // 2. Poll the socket briefly for the next server frame. WouldBlock
        //    (read timeout) is not fatal — just loop back to the queue drain.
        match ws.read() {
            Ok(tungstenite::Message::Text(text)) => match parse_server_frame(&text) {
                Some(Ok(frame)) => spawn_request(state, frame, backend),
                Some(Err(id)) => {
                    if let Some(flag) = lock_in_flight(&state.in_flight).remove(&id) {
                        flag.store(true, Ordering::SeqCst);
                    }
                }
                None => vlog("ignoring malformed frame from cloud"),
            },
            Ok(tungstenite::Message::Close(_)) => {
                vlog("server closed connection");
                return;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                continue;
            }
            Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
                return;
            }
            Err(err) => {
                vlog(&format!("ws error: {err}"));
                return;
            }
        }
    }
}

/// First up-to-`max_chars` characters of `s`, safe on any input (the id is
/// cloud-controlled and only used for diagnostics).
fn short_id(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// Spawn a worker thread for one incoming request frame and register its
/// cancel flag. An HTTP target is 'static: either a built-in probe constant or
/// a leaked --target value, resolved once per process.
fn lock_in_flight(
    map: &Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
) -> std::sync::MutexGuard<'_, BTreeMap<String, Arc<AtomicBool>>> {
    map.lock().unwrap_or_else(|e| e.into_inner())
}

fn spawn_request(state: &ConnState, frame: RequestFrame, backend: RelayBackend) {
    let cancel = Arc::new(AtomicBool::new(false));
    let id = frame.id.clone();
    lock_in_flight(&state.in_flight).insert(id.clone(), Arc::clone(&cancel));
    let tx = state.tx.clone();
    if let Err(err) = std::thread::Builder::new()
        .name(format!("relay-req-{}", short_id(&id, 8)))
        .spawn(move || match backend {
            RelayBackend::Http(target) => handle_request(&tx, &frame, target, &cancel),
            RelayBackend::Foundation => handle_foundation_request(&tx, &frame, &cancel),
        })
    {
        lock_in_flight(&state.in_flight).remove(&id);
        let _ = state.tx.send(ClientFrame::Error {
            id,
            message: format!("Could not spawn relay worker: {err}"),
        });
    }
}

fn abort_in_flight(in_flight: &Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>) {
    let mut map = lock_in_flight(in_flight);
    for flag in map.values() {
        flag.store(true, Ordering::SeqCst);
    }
    map.clear();
}

fn send_frame(ws: &mut Ws, frame: &ClientFrame) {
    if let Err(err) = ws.send(tungstenite::Message::text(frame.to_json())) {
        vlog(&format!("send {} failed: {err}", frame_type_name(frame)));
    }
}

fn frame_type_name(frame: &ClientFrame) -> &'static str {
    match frame {
        ClientFrame::Head { .. } => "head",
        ClientFrame::Chunk { .. } => "chunk",
        ClientFrame::Done { .. } => "done",
        ClientFrame::Error { .. } => "error",
        ClientFrame::Hello { .. } => "hello",
    }
}

/// Configure short socket read timeouts so `ws.read()` polls instead of
/// blocking forever (WouldBlock drives the main-loop multiplexing).
fn set_read_timeouts(ws: &Ws, timeout: Duration) {
    use tungstenite::stream::MaybeTlsStream;
    let sock: Option<&std::net::TcpStream> = match ws.get_ref() {
        MaybeTlsStream::Plain(s) => Some(s),
        MaybeTlsStream::Rustls(tls) => Some(&tls.sock),
        _ => None,
    };
    if let Some(sock) = sock {
        let _ = sock.set_read_timeout(Some(timeout));
        let _ = sock.set_write_timeout(Some(Duration::from_secs(30)));
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// `anyr relay start` — resolve credentials, detect target, connect forever.
fn run_relay_start(
    parsed: &crate::parse::ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let args = parse_start_args(parsed);
    VERBOSE.store(parsed.flag_true("verbose"), Ordering::Relaxed);

    let token = ensure_relay_token(args.token.as_deref(), &args.name, parsed, env)?;

    let backend = resolve_backend(args.target.as_deref());
    let target_label = match backend {
        RelayBackend::Foundation => crate::foundation::FOUNDATION_MODEL_ID,
        RelayBackend::Http(target) => target,
    };

    if args.pool {
        match resolve_relay_device_id(parsed, env)
            .and_then(|id| enable_relay_pool(&id, parsed, env))
        {
            Ok(()) => println!(
                "Pool sharing enabled — this device will serve other users' requests when your \
                 own capacity is idle, and you'll earn credits for each one. Manage it anytime: \
                 https://dash.anyrouter.dev/devices"
            ),
            Err(err) => {
                eprintln!("Could not enable pool sharing: {err}");
                eprintln!("Continuing without pool sharing — your own-device relay still works.");
            }
        }
    }

    let url = args
        .url
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(DEFAULT_WS_URL);
    println!("Starting relay: target={target_label} url={url}");

    let (tx, rx) = mpsc::channel();
    let conn = ConnState {
        tx,
        rx,
        in_flight: Arc::new(Mutex::new(BTreeMap::new())),
    };

    let mut attempt: u32 = 0;
    loop {
        match connect_ws(url, &token) {
            Ok(mut ws) => {
                attempt = 0;
                set_read_timeouts(&ws, Duration::from_millis(SOCKET_POLL_MS));
                serve_connection(&mut ws, backend, &conn, args.max_concurrency);
                abort_in_flight(&conn.in_flight);
            }
            Err(err) => {
                vlog(&err);
                ulog(&format!("relay connect failed: {err}"));
            }
        }
        attempt += 1;
        let delay = BASE_BACKOFF_MS
            .saturating_mul(1 << (attempt - 1).min(20))
            .min(MAX_BACKOFF_MS);
        vlog(&format!("reconnecting in {delay}ms"));
        std::thread::sleep(Duration::from_millis(delay));
    }
}

/// `anyr relay pair [--name <device>]` — explicit pairing (start auto-pairs).
fn run_relay_pair(
    parsed: &crate::parse::ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let name = crate::parse::get_string_flag(&parsed.flags, "name")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_DEVICE_NAME.into());
    let api_key = match resolve_sk_ar_key(parsed, env) {
        Some(k) => k,
        None => {
            return Err(format!(
                "Not logged in. Run: {} login (or just: {} relay start)",
                crate::help::invoked_bin(),
                crate::help::invoked_bin()
            ));
        }
    };
    let _ = pair_device(&api_key, &name, env)?;
    println!("Paired as \"{name}\". Token saved to your AnyRouter config.");
    println!("Run: {} relay start", crate::help::invoked_bin());
    Ok(0)
}

/// `anyr relay <start|pair>` dispatcher. Never returns from start — the relay
/// loop runs until the process is killed.
pub fn run(
    parsed: &crate::parse::ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let sub = parsed.passthrough.first().map(String::as_str);
    match sub {
        Some("start") => run_relay_start(parsed, env),
        Some("pair") => run_relay_pair(parsed, env),
        other => {
            let bin = crate::help::invoked_bin();
            if let Some(o) = other.filter(|o| !o.is_empty()) {
                eprintln!("Unknown relay subcommand: {o}");
            }
            eprintln!(
                "Usage: {bin} relay start [--target <url>] [--token <rk_…>] [--url <wss://…>]"
            );
            eprintln!(
                "               [--name <device>] [--pool] [--max-concurrency <n>] [--verbose]"
            );
            eprintln!("       (--target {})", detect_help_line());
            eprintln!(
                "       (--pool opts this device into the shared relay pool — earn credits when idle)"
            );
            eprintln!(r#"       {bin} relay pair --name "My Mac""#);
            Err(crate::cmd::dispatch::USAGE.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[test]
    fn dial_gives_up_within_the_timeout_on_a_blackholed_host() {
        // 192.0.2.0/24 (TEST-NET-1) is never routed: without a connect
        // timeout the reconnect loop would hang for minutes here.
        let t = std::time::Instant::now();
        let res = super::dial_with_timeout("192.0.2.1", 9, Duration::from_millis(300));
        assert!(res.is_err());
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "dial ignored its timeout"
        );
    }

    #[test]
    fn dial_connects_to_a_live_listener() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(super::dial_with_timeout("127.0.0.1", port, Duration::from_secs(2)).is_ok());
    }

    use super::*;

    fn request_frame(raw: &str) -> RequestFrame {
        parse_server_frame(raw)
            .expect("should parse")
            .expect("should be a request")
    }

    #[test]
    fn parses_request_frame_with_all_fields() {
        let f = request_frame(
            r#"{"type":"request","id":"abc","path":"/chat/completions","body":"{\"model\":\"m\"}","stream":true}"#,
        );
        assert_eq!(f.id, "abc");
        assert_eq!(f.path, "/chat/completions");
        assert_eq!(f.body, r#"{\"model\":\"m\"}"#.replace('\\', ""));
    }

    #[test]
    fn parses_request_without_optional_fields_as_empty() {
        let f = request_frame(r#"{"type":"request","id":"x"}"#);
        assert_eq!(f.id, "x");
        assert_eq!(f.path, "");
        assert_eq!(f.body, "");
    }

    #[test]
    fn cancel_frame_maps_to_err_with_id() {
        let parsed = parse_server_frame(r#"{"type":"cancel","id":"req-7"}"#).unwrap();
        assert_eq!(parsed.err().unwrap(), "req-7");
    }

    #[test]
    fn malformed_and_unknown_frames_are_ignored() {
        assert!(parse_server_frame("not json").is_none());
        assert!(parse_server_frame("{}").is_none());
        assert!(parse_server_frame(r#"{"type":"request"}"#).is_none()); // no id
        assert!(parse_server_frame(r#"{"id":"a"}"#).is_none()); // no type
        assert!(parse_server_frame(r#"{"type":"future-frame","id":"a"}"#).is_none());
        assert!(parse_server_frame("[1,2]").is_none());
    }

    #[test]
    fn head_frame_uses_camel_case_content_type() {
        let json = ClientFrame::Head {
            id: "r1".into(),
            status: 200,
            content_type: "text/event-stream".into(),
        }
        .to_json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "head");
        assert_eq!(v["id"], "r1");
        assert_eq!(v["status"], 200);
        assert_eq!(v["contentType"], "text/event-stream");
    }

    #[test]
    fn hello_frame_includes_max_concurrency_only_when_set() {
        let without = ClientFrame::Hello {
            models: vec!["foundation-model".into()],
            max_concurrency: None,
        }
        .to_json();
        let v: serde_json::Value = serde_json::from_str(&without).unwrap();
        assert_eq!(v["type"], "hello");
        assert_eq!(v["models"][0], "foundation-model");
        assert!(v.get("maxConcurrency").is_none());

        let with = ClientFrame::Hello {
            models: vec![],
            max_concurrency: Some(3),
        }
        .to_json();
        let v: serde_json::Value = serde_json::from_str(&with).unwrap();
        assert_eq!(v["maxConcurrency"], 3);
    }

    #[test]
    fn done_and_error_frames_are_minimal_envelopes() {
        let done: serde_json::Value =
            serde_json::from_str(&ClientFrame::Done { id: "d1".into() }.to_json()).unwrap();
        assert_eq!(done["type"], "done");
        assert_eq!(done["id"], "d1");

        let err: serde_json::Value = serde_json::from_str(
            &ClientFrame::Error {
                id: "e1".into(),
                message: "boom".into(),
            }
            .to_json(),
        )
        .unwrap();
        assert_eq!(err["type"], "error");
        assert_eq!(err["message"], "boom");
    }

    #[test]
    fn in_process_model_advertises_the_upstream_join_key() {
        // Pool routing joins on the upstream model name, even when the HTTP
        // catalog id is apple/foundation-model.
        let models = models_for_hello(RelayBackend::Foundation, &["llama3".into()]);
        assert_eq!(models, vec!["foundation-model"]);
        assert!(!models.iter().any(|m| m == "apple/foundation-model"));
    }

    #[test]
    fn http_targets_advertise_the_server_model_list() {
        // :8000 and Ollama are not the Apple model. Their hello list is
        // exactly what /v1/models returned.
        let fetched = vec!["llama3".into(), "qwen2".into()];
        let models = models_for_hello(RelayBackend::Http("http://10.0.0.5:8000/v1"), &fetched);
        assert_eq!(models, fetched);
        let models = models_for_hello(RelayBackend::Http(OLLAMA_TARGET), &fetched);
        assert_eq!(models, fetched);
        assert!(!models.iter().any(|m| m == "foundation-model"));
    }

    #[test]
    fn fm_serve_advertises_the_pool_join_key() {
        // fm serve lists `system` (older builds listed `_base`). Pool routing
        // joins on foundation-model, so the hello frame has to add it.
        let models = models_for_hello(RelayBackend::Http(FM_SERVE_TARGET), &["system".into()]);
        assert_eq!(models, vec!["system", "foundation-model"]);
        let models = models_for_hello(RelayBackend::Http("http://localhost:1976/v1"), &[]);
        assert_eq!(models, vec!["foundation-model"]);
        let models = models_for_hello(
            RelayBackend::Http(FM_SERVE_TARGET),
            &["foundation-model".into()],
        );
        assert_eq!(models, vec!["foundation-model"]);
        assert!(!models.iter().any(|m| m == "apple/foundation-model"));
    }

    #[test]
    fn auto_detect_probes_fm_serve_then_port_8000_then_ollama() {
        let targets: Vec<_> = RELAY_PROBES.iter().map(|p| p.target).collect();
        assert_eq!(
            targets,
            vec![FM_SERVE_TARGET, DEFAULT_TARGET, OLLAMA_TARGET]
        );
    }

    #[test]
    fn fm_serve_request_rewrites_the_pool_join_key_to_the_local_model_id() {
        let out = body_for_http_target(
            FM_SERVE_TARGET,
            r#"{"model":"foundation-model","messages":[{"role":"user","content":"hi"}]}"#,
        );
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["model"], "system");
        assert_eq!(value["messages"][0]["content"], "hi");

        let out = body_for_http_target(
            "http://localhost:1976/v1",
            r#"{"model":"apple/foundation-model"}"#,
        );
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["model"], "system");

        let ollama = r#"{"model":"llama3"}"#;
        assert_eq!(body_for_http_target(OLLAMA_TARGET, ollama), ollama);
        let already = r#"{"model":"system"}"#;
        assert_eq!(body_for_http_target(FM_SERVE_TARGET, already), already);
        assert_eq!(
            body_for_http_target(FM_SERVE_TARGET, "not-json"),
            "not-json"
        );
    }

    #[test]
    fn explicit_target_wins_over_an_available_system_model() {
        match choose_backend(
            Some(" http://127.0.0.1:9/v1 "),
            true,
            "http://localhost:11434/v1",
        ) {
            RelayBackend::Http(target) => assert_eq!(target, "http://127.0.0.1:9/v1"),
            RelayBackend::Foundation => panic!("explicit --target must select that server"),
        }
    }

    #[test]
    fn available_system_model_is_chosen_before_http_probes() {
        assert_eq!(
            choose_backend(None, true, OLLAMA_TARGET),
            RelayBackend::Foundation
        );
        assert_eq!(
            choose_backend(Some("  "), false, OLLAMA_TARGET),
            RelayBackend::Http(OLLAMA_TARGET)
        );
    }

    #[test]
    fn foundation_request_with_a_bad_body_does_not_call_the_model() {
        let (tx, rx) = mpsc::channel();
        handle_foundation_request(
            &tx,
            &req("r9", "/chat/completions", "not-json"),
            &AtomicBool::new(false),
        );
        let frames = drain(&rx);
        match frames.as_slice() {
            [ClientFrame::Error { id, message }] => {
                assert_eq!(id, "r9");
                assert!(message.contains("not JSON"), "{message}");
            }
            _ => panic!("expected one error frame, got {} frames", frames.len()),
        }
    }

    #[test]
    fn foundation_request_rejects_a_path_other_than_chat_completions() {
        let (tx, rx) = mpsc::channel();
        handle_foundation_request(
            &tx,
            &req("r10", "/v1/models", "{}"),
            &AtomicBool::new(false),
        );
        let frames = drain(&rx);
        match frames.as_slice() {
            [ClientFrame::Error { id, message }] => {
                assert_eq!(id, "r10");
                assert!(message.contains("/v1/models"), "{message}");
            }
            _ => panic!("expected one error frame, got {} frames", frames.len()),
        }
    }

    #[test]
    fn short_id_is_char_boundary_safe_and_bounded() {
        assert_eq!(short_id("abcdefghijklmnop", 8), "abcdefgh");
        // Multibyte inside the first 8 BYTES must not panic.
        assert_eq!(short_id("héllo-world", 4), "héll");
        assert_eq!(short_id("🚀🚀🚀", 2), "🚀🚀");
        assert_eq!(short_id("", 8), "");
    }

    #[test]
    fn utf8_flush_len_never_splits_a_codepoint() {
        // "héllo" where é is 2 bytes: split inside é keeps the partial byte.
        let bytes = "héllo".as_bytes();
        assert_eq!(utf8_flush_len(&bytes[..1]), 1); // 'h' complete, é split
        assert_eq!(utf8_flush_len(bytes), 6); // whole string

        // 4-byte emoji split mid-sequence.
        let rocket = "🚀x";
        let rb = rocket.as_bytes();
        assert_eq!(utf8_flush_len(&rb[..2]), 0);
        assert_eq!(utf8_flush_len(rb), 5);

        // ASCII flushes everything.
        assert_eq!(utf8_flush_len(b"plain"), 5);
    }

    #[test]
    fn start_args_read_flags_with_defaults() {
        let mk = |flags: Vec<(&str, crate::parse::FlagValue)>| crate::parse::ParsedArgs {
            command: "relay".into(),
            flags: flags.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            passthrough: vec!["start".into()],
        };
        use crate::parse::FlagValue;

        let bare = mk(vec![("verbose", FlagValue::Bool(true))]);
        let args = parse_start_args(&bare);
        assert!(args.target.is_none());
        assert_eq!(args.name, DEFAULT_DEVICE_NAME);
        assert!(!args.pool);
        assert!(args.max_concurrency.is_none());

        let full = mk(vec![
            (
                "target",
                FlagValue::Value("http://127.0.0.1:1976/v1".into()),
            ),
            ("token", FlagValue::Value("rk_x".into())),
            ("url", FlagValue::Value("wss://example.test/relay".into())),
            ("name", FlagValue::Value("Desk".into())),
            ("pool", FlagValue::Bool(true)),
            ("max-concurrency", FlagValue::Value("4".into())),
        ]);
        let args = parse_start_args(&full);
        assert_eq!(args.target.as_deref(), Some("http://127.0.0.1:1976/v1"));
        assert_eq!(args.token.as_deref(), Some("rk_x"));
        assert_eq!(args.name, "Desk");
        assert!(args.pool);
        assert_eq!(args.max_concurrency, Some(4));

        // Non-positive / non-numeric max-concurrency is ignored, not an error.
        let bad = mk(vec![("max-concurrency", FlagValue::Value("0".into()))]);
        assert!(parse_start_args(&bad).max_concurrency.is_none());
    }

    #[test]
    fn device_login_base_strips_v1_suffix() {
        assert_eq!(device_login_base(), "https://anyrouter.dev/api");
    }

    #[test]
    fn config_round_trips_relay_fields() {
        let src = "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-real
    relay_token: rk_pairing-token
    relay_device_id: dev_123
";
        let cfg = crate::config::parse_config(src);
        let p = cfg.profiles.get("default").unwrap();
        assert_eq!(p.relay_token.as_deref(), Some("rk_pairing-token"));
        assert_eq!(p.relay_device_id.as_deref(), Some("dev_123"));

        let again = crate::config::parse_config(&crate::config::serialize_config(&cfg));
        let p = again.profiles.get("default").unwrap();
        assert_eq!(p.relay_token.as_deref(), Some("rk_pairing-token"));
        assert_eq!(p.relay_device_id.as_deref(), Some("dev_123"));
    }

    fn spawn_stub_target(responses: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
        let addr = listener.local_addr().expect("local addr");
        let url = format!("http://{addr}");
        let handle = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                let header_end = pos + 4;
                                let headers = String::from_utf8_lossy(&buf[..header_end]);
                                let want = headers
                                    .lines()
                                    .find_map(|line| {
                                        let (k, v) = line.split_once(':')?;
                                        (k.eq_ignore_ascii_case("content-length"))
                                            .then(|| v.trim().parse::<usize>().ok())
                                            .flatten()
                                    })
                                    .unwrap_or(0);
                                while buf.len() - header_end < want {
                                    match stream.read(&mut tmp) {
                                        Ok(0) => break,
                                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                                        Err(_) => break,
                                    }
                                }
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let reason = match status {
                    200 => "OK",
                    502 => "Bad Gateway",
                    _ => "OK",
                };
                let bytes = body.as_bytes();
                let resp = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(bytes);
                let _ = stream.flush();
            }
        });
        (url, handle)
    }

    fn req(id: &str, path: &str, body: &str) -> RequestFrame {
        RequestFrame {
            id: id.into(),
            path: path.into(),
            body: body.into(),
        }
    }

    fn drain(rx: &mpsc::Receiver<ClientFrame>) -> Vec<ClientFrame> {
        let mut out = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            out.push(frame);
        }
        out
    }

    fn chunk_text(frames: &[ClientFrame]) -> String {
        frames
            .iter()
            .filter_map(|f| match f {
                ClientFrame::Chunk { data, .. } => Some(data.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn streams_local_response_back_as_head_chunk_done() {
        let (target, join) = spawn_stub_target(vec![(200, "hello world".into())]);
        let (tx, rx) = mpsc::channel();
        handle_request(
            &tx,
            &req("r1", "/v1/chat/completions", "{}"),
            &target,
            &AtomicBool::new(false),
        );
        let frames = drain(&rx);
        let _ = join.join();
        assert!(
            matches!(
                frames.as_slice(),
                [
                    ClientFrame::Head {
                        id,
                        status: 200,
                        ..
                    },
                    ClientFrame::Chunk { data, .. },
                    ClientFrame::Done { .. }
                ] if id == "r1" && data == "hello world"
            ),
            "expected Head(200)+Chunk+Done, got {} frames",
            frames.len()
        );
    }

    #[test]
    fn non_2xx_from_target_relays_status_and_body() {
        let (target, join) = spawn_stub_target(vec![(502, "bad gateway".into())]);
        let (tx, rx) = mpsc::channel();
        handle_request(
            &tx,
            &req("r2", "/v1/chat/completions", "{}"),
            &target,
            &AtomicBool::new(false),
        );
        let frames = drain(&rx);
        let _ = join.join();
        assert!(
            matches!(
                frames.as_slice(),
                [
                    ClientFrame::Head { status: 502, .. },
                    ClientFrame::Chunk { data, .. },
                    ClientFrame::Done { .. }
                ] if data == "bad gateway"
            ),
            "expected Head(502)+Chunk+Done, got {} frames",
            frames.len()
        );
    }

    #[test]
    fn unreachable_target_produces_error_frame() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let target = format!("http://{addr}");
        let (tx, rx) = mpsc::channel();
        handle_request(
            &tx,
            &req("r3", "/v1/chat/completions", "{}"),
            &target,
            &AtomicBool::new(false),
        );
        let frames = drain(&rx);
        assert_eq!(frames.len(), 1, "expected a single Error frame");
        match &frames[0] {
            ClientFrame::Error { id, message } => {
                assert_eq!(id, "r3");
                assert!(message.contains("Local target unreachable"), "{message}");
            }
            _ => panic!("expected Error frame"),
        }
    }

    #[test]
    fn cancel_flag_stops_streaming() {
        let body = "x".repeat(1024 * 1024);
        let (target, join) = spawn_stub_target(vec![(200, body)]);
        let (tx, rx) = mpsc::channel();
        handle_request(
            &tx,
            &req("r4", "/v1/chat/completions", "{}"),
            &target,
            &AtomicBool::new(true),
        );
        let frames = drain(&rx);
        let _ = join.join();
        let extra = frames
            .iter()
            .any(|f| matches!(f, ClientFrame::Chunk { .. } | ClientFrame::Done { .. }));
        assert!(
            !extra,
            "cancel before handle_request must not send Chunk or Done (got {} frames)",
            frames.len()
        );
    }

    #[test]
    fn utf8_split_body_round_trips() {
        let expected = "héllo 🚀".repeat(1000);
        let (target, join) = spawn_stub_target(vec![(200, expected.clone())]);
        let (tx, rx) = mpsc::channel();
        handle_request(
            &tx,
            &req("r5", "/v1/chat/completions", "{}"),
            &target,
            &AtomicBool::new(false),
        );
        let frames = drain(&rx);
        let _ = join.join();
        assert!(
            matches!(frames.first(), Some(ClientFrame::Head { status: 200, .. })),
            "missing Head(200)"
        );
        assert!(
            matches!(frames.last(), Some(ClientFrame::Done { .. })),
            "missing Done"
        );
        assert_eq!(chunk_text(&frames), expected);
    }
}
