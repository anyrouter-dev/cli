//! `anyr relay start` against a mock cloud that misbehaves the ways a real
//! network does: a clean close, a half-open socket, an outage, a drop with a
//! request in flight, a host that never resolves, and a cloud that stops
//! reading. Each test drives the real binary and asserts what reaches the
//! cloud and the local model server.
//!
//! `ANYR_RELAY_TIME_SCALE` shrinks the relay's ping, liveness and backoff
//! timings so these run in seconds; 0.1 turns the 25s liveness window into
//! 2.5s and the 30s backoff cap into 3s.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tungstenite::{Message, WebSocket};

const KEY: &str = "sk-ar-v1-e2etestkey0123456789abcdef";
const SCALE: &str = "0.1";

fn temp_home(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "anyr-relay-res-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A foreground relay, killed when dropped so a failed assertion leaves no
/// process behind.
struct Relay {
    child: Child,
    log: PathBuf,
}

impl Relay {
    fn start(bin: &Path, ws_url: &str, target: &str, scale: &str) -> Relay {
        let home = temp_home("home");
        let log = home.join("relay.log");
        let child = Command::new(bin)
            .args([
                "relay",
                "start",
                "--no-pool",
                "--no-launch",
                "--base-url",
                "http://127.0.0.1:1/api",
                "--url",
                ws_url,
                "--target",
                target,
                "--key",
                KEY,
            ])
            .env("ANYR_NO_UPDATE", "1")
            .env("ANYR_NO_CATALOG", "1")
            .env("NO_COLOR", "1")
            .env("ANYROUTER_HOME", &home)
            .env("ANYR_RELAY_TIME_SCALE", scale)
            .env_remove("ANYROUTER_API_KEY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .expect("spawn relay");
        Relay { child, log }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bin() -> PathBuf {
    std::env::var_os("ANYR_RELAY_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_anyr")))
}

// ---------------------------------------------------------------------------
// Local model server
// ---------------------------------------------------------------------------

/// How the local model server answers a completion.
#[derive(Clone, Copy)]
struct Answer {
    /// Wait before the status line, like a model loading its weights.
    head_after: Duration,
    chunks: usize,
    chunk_bytes: usize,
    chunk_every: Duration,
}

/// Local OpenAI-compatible server. Records, per completion, how long after
/// its head the relay hung up (None while the relay keeps reading).
struct Target {
    url: String,
    hangups: Arc<Mutex<Vec<Option<Duration>>>>,
}

fn target(answer: Answer) -> Target {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let hangups: Arc<Mutex<Vec<Option<Duration>>>> = Arc::default();
    let hangups_in = Arc::clone(&hangups);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let hangups = Arc::clone(&hangups_in);
            std::thread::spawn(move || serve_target(stream, answer, &hangups));
        }
    });
    Target { url, hangups }
}

fn serve_target(mut stream: TcpStream, a: Answer, hangups: &Mutex<Vec<Option<Duration>>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len];
    let _ = reader.read_exact(&mut body);
    if line.starts_with("GET") {
        let models = r#"{"data":[{"id":"stub-model"}]}"#;
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{models}",
            models.len()
        );
        return;
    }
    let slot = {
        let mut h = hangups.lock().unwrap();
        h.push(None);
        h.len() - 1
    };
    std::thread::sleep(a.head_after);
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
    let started = Instant::now();
    let chunk = format!("data: {}\n\n", "a".repeat(a.chunk_bytes));
    let mut wrote = stream.write_all(head.as_bytes()).is_ok();
    for _ in 0..a.chunks {
        if !wrote {
            break;
        }
        std::thread::sleep(a.chunk_every);
        wrote = stream.write_all(chunk.as_bytes()).is_ok();
    }
    if !wrote {
        hangups.lock().unwrap()[slot] = Some(started.elapsed());
    }
}

// ---------------------------------------------------------------------------
// Mock cloud
// ---------------------------------------------------------------------------

type Ws = WebSocket<TcpStream>;

/// One relay socket the cloud accepted.
#[derive(Clone, Debug)]
struct Accepted {
    at: Instant,
    uri: String,
}

/// Mock cloud: accepts relay sockets on `addr` and runs `script` for each
/// one on its own thread, with the connection's index.
struct Cloud {
    addr: SocketAddr,
    accepted: Arc<Mutex<Vec<Accepted>>>,
}

impl Cloud {
    fn url(&self) -> String {
        format!("ws://{}/api/v1/relay/connect", self.addr)
    }

    fn start(script: impl Fn(usize, Ws) + Send + Sync + 'static) -> Cloud {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted: Arc<Mutex<Vec<Accepted>>> = Arc::default();
        let script = Arc::new(script);
        let acc = Arc::clone(&accepted);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let Some((n, ws)) = accept(stream, &acc) else {
                    continue;
                };
                let script = Arc::clone(&script);
                std::thread::spawn(move || script(n, ws));
            }
        });
        Cloud { addr, accepted }
    }

    fn accepted(&self) -> Vec<Accepted> {
        self.accepted.lock().unwrap().clone()
    }

    /// Block until `n` sockets were accepted, or panic after `within`.
    fn wait_for(&self, n: usize, within: Duration) -> Vec<Accepted> {
        let deadline = Instant::now() + within;
        loop {
            let seen = self.accepted();
            if seen.len() >= n {
                return seen;
            }
            assert!(
                Instant::now() < deadline,
                "relay opened {} socket(s), expected {n} within {within:?}: {seen:?}",
                seen.len()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn accept(stream: TcpStream, accepted: &Mutex<Vec<Accepted>>) -> Option<(usize, Ws)> {
    let uri = Arc::new(Mutex::new(String::new()));
    let uri_in = Arc::clone(&uri);
    #[allow(clippy::result_large_err)]
    let on_handshake =
        move |req: &tungstenite::handshake::server::Request,
              resp: tungstenite::handshake::server::Response| {
            *uri_in.lock().unwrap() = req.uri().to_string();
            Ok(resp)
        };
    let ws = tungstenite::accept_hdr(stream, on_handshake).ok()?;
    let mut acc = accepted.lock().unwrap();
    acc.push(Accepted {
        at: Instant::now(),
        uri: uri.lock().unwrap().clone(),
    });
    Some((acc.len() - 1, ws))
}

/// Read the hello and bind the socket to `dev-1`, as the real cloud does.
fn greet(ws: &mut Ws) {
    let _ = ws.get_ref().set_read_timeout(Some(Duration::from_secs(10)));
    let hello = ws.read().expect("hello");
    assert!(hello.to_text().unwrap_or("").contains("\"hello\""));
    ws.send(Message::text(
        r#"{"type":"device","id":"dev-1","name":"Desk"}"#,
    ))
    .unwrap();
}

fn send_request(ws: &mut Ws, id: &str, stream: bool) {
    let body = r#"{"model":"stub-model","messages":[{"role":"user","content":"hi"}]}"#;
    ws.send(Message::text(
        serde_json::json!({"type":"request","id":id,"path":"/chat/completions","body":body,"stream":stream})
            .to_string(),
    ))
    .unwrap();
}

/// Every JSON frame the relay sends within `window`.
fn frames_for(ws: &mut Ws, window: Duration) -> Vec<Value> {
    let _ = ws
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(100)));
    let deadline = Instant::now() + window;
    let mut out = Vec::new();
    while Instant::now() < deadline {
        match ws.read() {
            Ok(Message::Text(t)) => {
                if let Ok(v) = serde_json::from_str::<Value>(&t) {
                    out.push(v);
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    out
}

/// Hold the socket open, reading (and so answering pings) until it dies.
fn idle(mut ws: Ws) {
    let _ = ws.get_ref().set_read_timeout(None);
    while ws.read().is_ok() {}
}

fn types_for<'a>(frames: &'a [Value], id: &str) -> Vec<&'a str> {
    frames
        .iter()
        .filter(|f| f["id"] == id)
        .filter_map(|f| f["type"].as_str())
        .collect()
}

/// Each socket after the first reuses the device the cloud named, so a
/// reconnect never mints a new device row.
fn assert_same_device(seen: &[Accepted]) {
    assert!(seen[0].uri.contains("fresh=1"), "{}", seen[0].uri);
    for a in &seen[1..] {
        assert!(a.uri.contains("device=dev-1"), "{}", a.uri);
        assert!(!a.uri.contains("fresh=1"), "{}", a.uri);
    }
}

const QUICK: Answer = Answer {
    head_after: Duration::ZERO,
    chunks: 1,
    chunk_bytes: 16,
    chunk_every: Duration::ZERO,
};

// ---------------------------------------------------------------------------
// (a) clean close
// ---------------------------------------------------------------------------

/// WHY: a cloud that accepts and then closes at once (deploy, eviction, a
/// bad edge) used to be retried every 1s forever, because the backoff reset
/// on every handshake. The delay now grows until a socket stays up.
#[test]
fn a_clean_close_reconnects_at_once_and_backs_off_while_flapping() {
    let local = target(QUICK);
    let cloud = Cloud::start(|_, mut ws| {
        greet(&mut ws);
        std::thread::sleep(Duration::from_millis(50));
        let _ = ws.close(None);
        let _ = ws.flush();
        idle(ws);
    });
    let relay = Relay::start(&bin(), &cloud.url(), &local.url, SCALE);
    let seen = cloud.wait_for(6, Duration::from_secs(30));
    let gaps: Vec<u128> = seen
        .windows(2)
        .map(|w| (w[1].at - w[0].at).as_millis())
        .collect();
    eprintln!("measure a: reconnect gaps ms = {gaps:?}");
    assert_same_device(&seen);
    // Scaled backoff: base 100ms, so the first retry is quick.
    assert!(gaps[0] < 1_000, "first reconnect too slow: {gaps:?}");
    assert!(
        gaps[4] >= 2 * gaps[0],
        "backoff did not grow while flapping: {gaps:?}\n{}",
        relay.log()
    );
}

// ---------------------------------------------------------------------------
// (b) half-open socket
// ---------------------------------------------------------------------------

/// WHY: a socket whose peer stops answering (sleeping laptop, NAT timeout,
/// dead edge) never errors locally. Without a liveness check the relay sat
/// on it for hours, and the request on it hung until the cloud gave up.
#[test]
fn b_half_open_socket_is_dropped_and_its_request_fails_on_the_next_socket() {
    let local = target(Answer {
        head_after: Duration::ZERO,
        chunks: 600,
        chunk_bytes: 16,
        chunk_every: Duration::from_millis(100),
    });
    let next: Arc<Mutex<Option<Vec<Value>>>> = Arc::default();
    let next_in = Arc::clone(&next);
    let cloud = Cloud::start(move |n, mut ws| {
        greet(&mut ws);
        if n == 0 {
            send_request(&mut ws, "req-1", true);
            // Silent from here on: no reads, so no pongs, and no close.
            std::thread::sleep(Duration::from_secs(120));
            return;
        }
        if n == 1 {
            let frames = frames_for(&mut ws, Duration::from_secs(2));
            *next_in.lock().unwrap() = Some(frames);
        }
        idle(ws);
    });
    let relay = Relay::start(&bin(), &cloud.url(), &local.url, SCALE);
    cloud.wait_for(1, Duration::from_secs(10));
    let seen = cloud.wait_for(2, Duration::from_secs(20));
    let detect = seen[1].at - seen[0].at;
    eprintln!("measure b: half-open detected and replaced after {detect:?}");
    assert_same_device(&seen);
    // Liveness window 2.5s scaled, plus the first backoff step.
    assert!(
        detect < Duration::from_secs(6),
        "{detect:?}\n{}",
        relay.log()
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while next.lock().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let frames = next.lock().unwrap().clone().expect("second socket read");
    assert_eq!(
        types_for(&frames, "req-1"),
        ["error"],
        "the cloud must learn the request died: {frames:?}"
    );
    let hangups = local.hangups.lock().unwrap().clone();
    assert!(
        matches!(hangups.as_slice(), [Some(_)]),
        "the local model call kept running: {hangups:?}"
    );
}

// ---------------------------------------------------------------------------
// (c) cloud unreachable, then back
// ---------------------------------------------------------------------------

/// WHY: an outage of a minute or two must end in a reconnect on its own,
/// within one capped backoff step of the cloud coming back.
///
/// The cloud lives on 127.0.0.2 (Linux routes all of 127/8 to loopback), so
/// no other test's mock can take its port during the outage.
#[cfg(target_os = "linux")]
#[test]
fn c_relay_returns_within_one_capped_step_after_an_outage() {
    let local = target(QUICK);
    let listener = TcpListener::bind("127.0.0.2:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("ws://{addr}/api/v1/relay/connect");
    let accepted: Arc<Mutex<Vec<Accepted>>> = Arc::default();
    let relay = Relay::start(&bin(), &url, &local.url, SCALE);

    let (_, mut ws) = accept(listener.accept().unwrap().0, &accepted).unwrap();
    greet(&mut ws);
    drop(listener);
    drop(ws);
    // 5s scaled is 50s of real outage: every retry hits a closed port.
    std::thread::sleep(Duration::from_secs(5));
    let listener = TcpListener::bind(addr).unwrap();
    let back = Instant::now();
    listener.set_nonblocking(true).unwrap();
    let deadline = back + Duration::from_secs(15);
    let stream = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("relay never came back: {e}\n{}", relay.log()),
        }
    };
    stream.set_nonblocking(false).unwrap();
    let returned = back.elapsed();
    let (_, mut ws) = accept(stream, &accepted).unwrap();
    greet(&mut ws);
    eprintln!("measure c: back online {returned:?} after the cloud returned");
    assert_same_device(&accepted.lock().unwrap());
    // Cap 3s scaled, plus slack for the probe of the local target.
    assert!(returned < Duration::from_secs(5), "{returned:?}");
}

// ---------------------------------------------------------------------------
// (d) request in flight when the socket drops
// ---------------------------------------------------------------------------

/// WHY: frames of a request from a dropped socket used to go out on the next
/// one (one queue for every socket), and the cloud never heard that the
/// request died. The next socket now carries one `error` for it and nothing
/// else, and the local call is abandoned.
#[test]
fn d_request_in_flight_on_a_dropped_socket_errors_once_and_never_leaks() {
    let local = target(Answer {
        head_after: Duration::from_millis(1500),
        chunks: 100,
        chunk_bytes: 16,
        chunk_every: Duration::from_millis(100),
    });
    let next: Arc<Mutex<Option<Vec<Value>>>> = Arc::default();
    let next_in = Arc::clone(&next);
    let cloud = Cloud::start(move |n, mut ws| {
        greet(&mut ws);
        if n == 0 {
            send_request(&mut ws, "req-1", false);
            std::thread::sleep(Duration::from_millis(300));
            let _ = ws.close(None);
            let _ = ws.flush();
            return;
        }
        if n == 1 {
            let frames = frames_for(&mut ws, Duration::from_secs(4));
            *next_in.lock().unwrap() = Some(frames);
        }
        idle(ws);
    });
    let relay = Relay::start(&bin(), &cloud.url(), &local.url, SCALE);
    let seen = cloud.wait_for(2, Duration::from_secs(10));
    assert_same_device(&seen);
    let deadline = Instant::now() + Duration::from_secs(8);
    while next.lock().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let frames = next.lock().unwrap().clone().expect("second socket read");
    let types = types_for(&frames, "req-1");
    let hangups = local.hangups.lock().unwrap().clone();
    eprintln!("measure d: req-1 frames on the next socket = {types:?}, local hangup = {hangups:?}");
    assert_eq!(types, ["error"], "{frames:?}\n{}", relay.log());
    assert!(
        matches!(hangups.as_slice(), [Some(d)] if *d < Duration::from_secs(2)),
        "the local model call kept running: {hangups:?}"
    );
}

// ---------------------------------------------------------------------------
// (e) DNS and connect errors
// ---------------------------------------------------------------------------

/// WHY: a host that does not resolve or refuses the connection is a network
/// problem, not a bad credential: the relay keeps retrying, logs the cause
/// once, and does not exit.
#[test]
fn e_unresolvable_and_refusing_clouds_keep_the_relay_retrying() {
    let local = target(QUICK);
    // Port 1 refuses on loopback and is never handed out by bind(0), so no
    // other test's mock can end up there. The trailing dot keeps the
    // resolver from trying search domains.
    for url in [
        "ws://relay.invalid./api/v1/relay/connect",
        "ws://127.0.0.1:1/api/v1/relay/connect",
    ] {
        let mut relay = Relay::start(&bin(), url, &local.url, SCALE);
        std::thread::sleep(Duration::from_secs(4));
        assert!(
            relay.child.try_wait().unwrap().is_none(),
            "relay exited on {url}:\n{}",
            relay.log()
        );
        let log = relay.log();
        let failures = log.matches("relay connect failed").count();
        let retries = log.matches("reconnecting in").count();
        eprintln!("measure e: {url}: {failures} distinct failure line(s), {retries} retry line(s)");
        assert_eq!(failures, 1, "{log}");
        assert!(retries >= 3, "{log}");
    }
}

/// WHY: a host that accepts the TCP connection but never answers the
/// upgrade (an overloaded edge, a captive portal) used to block the
/// handshake forever, with no read timeout. The relay now gives up after
/// the 15s connect timeout (not scaled) and retries.
#[test]
fn e_a_cloud_that_never_answers_the_upgrade_is_retried() {
    let local = target(QUICK);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "ws://{}/api/v1/relay/connect",
        listener.local_addr().unwrap()
    );
    let opened: Arc<Mutex<Vec<Instant>>> = Arc::default();
    let opened_in = Arc::clone(&opened);
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            opened_in.lock().unwrap().push(Instant::now());
            held.push(stream);
        }
    });
    let relay = Relay::start(&bin(), &url, &local.url, SCALE);
    let deadline = Instant::now() + Duration::from_secs(25);
    while opened.lock().unwrap().len() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let opened = opened.lock().unwrap().clone();
    assert!(
        opened.len() >= 2,
        "relay stuck in the first handshake:\n{}",
        relay.log()
    );
    eprintln!(
        "measure e: silent upgrade abandoned and retried after {:?}",
        opened[1] - opened[0]
    );
}

// ---------------------------------------------------------------------------
// Long streams
// ---------------------------------------------------------------------------

/// WHY: a long stream to a slow cloud must not look like a dead socket. The
/// relay has to read while its queue stays full, and the pong must not wait
/// behind megabytes the kernel buffered ahead of the ping.
#[cfg(target_os = "linux")]
#[test]
fn g_a_long_stream_to_a_slow_cloud_is_not_taken_for_a_dead_socket() {
    let local = target(Answer {
        head_after: Duration::ZERO,
        chunks: 600,
        chunk_bytes: 8 * 1024,
        chunk_every: Duration::ZERO,
    });
    let finished: Arc<Mutex<Option<(String, Duration)>>> = Arc::default();
    let finished_in = Arc::clone(&finished);
    let cloud = Cloud::start(move |n, mut ws| {
        greet(&mut ws);
        if n > 0 {
            idle(ws);
            return;
        }
        // A slow link: a modest window drained 8 KiB every 10 ms (0.8 MB/s),
        // slower than an autotuned multi-megabyte send buffer can empty
        // within the liveness window.
        set_receive_buffer(ws.get_ref(), 256 * 1024);
        send_request(&mut ws, "req-1", true);
        let started = Instant::now();
        let _ = ws.get_ref().set_read_timeout(Some(Duration::from_secs(10)));
        let mut last = String::from("none");
        while let Ok(msg) = ws.read() {
            std::thread::sleep(Duration::from_millis(10));
            let v: Value = serde_json::from_str(msg.to_text().unwrap_or("")).unwrap_or_default();
            if v["type"] == "done" || v["type"] == "error" {
                last = v["type"].as_str().unwrap_or("").to_string();
                break;
            }
        }
        *finished_in.lock().unwrap() = Some((last, started.elapsed()));
        idle(ws);
    });
    let relay = Relay::start(&bin(), &cloud.url(), &local.url, SCALE);
    let deadline = Instant::now() + Duration::from_secs(30);
    while finished.lock().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let (last, took) = finished
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| panic!("stream never finished:\n{}", relay.log()));
    std::thread::sleep(Duration::from_secs(1));
    let sockets = cloud.accepted().len();
    eprintln!("measure g: 5 MB stream ended with {last} after {took:?}, sockets opened {sockets}");
    assert_eq!(last, "done", "{}", relay.log());
    assert!(
        took > Duration::from_secs(3),
        "stream too fast to test: {took:?}"
    );
    assert_eq!(sockets, 1, "relay dropped a live socket:\n{}", relay.log());
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn status_kb(pid: u32, field: &str) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix(field))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

/// WHY: when the cloud stops reading, the local stream used to pile up in an
/// unbounded queue: the whole response sat in the relay's memory. The queue
/// is bounded now, so the local read waits instead.
#[cfg(target_os = "linux")]
#[test]
fn f_a_stalled_cloud_does_not_buffer_the_local_stream_in_memory() {
    let local = target(Answer {
        head_after: Duration::ZERO,
        chunks: 2_000,
        chunk_bytes: 64 * 1024,
        chunk_every: Duration::ZERO,
    });
    let cloud = Cloud::start(|n, mut ws| {
        greet(&mut ws);
        if n == 0 {
            std::thread::sleep(Duration::from_millis(1000));
            set_receive_buffer(ws.get_ref(), 4096);
            send_request(&mut ws, "req-1", true);
            std::thread::sleep(Duration::from_secs(120));
            return;
        }
        idle(ws);
    });
    let relay = Relay::start(&bin(), &cloud.url(), &local.url, SCALE);
    cloud.wait_for(1, Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(500));
    let base = status_kb(relay.pid(), "VmRSS:");
    std::thread::sleep(Duration::from_millis(2500));
    let peak = status_kb(relay.pid(), "VmHWM:");
    eprintln!("measure f: RSS before {base} kB, peak while stalled {peak} kB (128 MB streamed)");
    assert!(
        peak < base + 24 * 1024,
        "relay buffered the stream: {base} kB -> {peak} kB"
    );
}

#[cfg(target_os = "linux")]
fn set_receive_buffer(s: &TcpStream, bytes: libc::c_int) {
    use std::os::fd::AsRawFd;
    let size = bytes;
    unsafe {
        libc::setsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&size as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
}

/// Soak: many reconnects, each serving one streamed request. Prints RSS
/// every 100 cycles. Run with
/// `cargo test --test relay_resilience soak -- --ignored --nocapture`;
/// `ANYR_RELAY_BIN` points it at another build, which must be a debug build
/// (release builds ignore `ANYR_RELAY_TIME_SCALE`); `ANYR_SOAK_CYCLES` and
/// `ANYR_SOAK_SCALE` size it, and `ANYR_SOAK_LARGE=1` makes each answer a
/// 1 MB stream.
#[cfg(target_os = "linux")]
#[test]
#[ignore]
fn soak_rss_stays_flat_across_reconnects() {
    let cycles: usize = std::env::var("ANYR_SOAK_CYCLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let scale = std::env::var("ANYR_SOAK_SCALE").unwrap_or_else(|_| "0.01".into());
    let small = target(QUICK);
    let large = target(Answer {
        head_after: Duration::ZERO,
        chunks: 16,
        chunk_bytes: 64 * 1024,
        chunk_every: Duration::ZERO,
    });
    let served: Arc<Mutex<usize>> = Arc::default();
    let served_in = Arc::clone(&served);
    let cloud = Cloud::start(move |n, mut ws| {
        greet(&mut ws);
        send_request(&mut ws, &format!("req-{n}"), true);
        let frames = frames_for_done(&mut ws, Duration::from_secs(10));
        if frames {
            *served_in.lock().unwrap() += 1;
        }
        let _ = ws.close(None);
        let _ = ws.flush();
        idle(ws);
    });
    let url = if std::env::var("ANYR_SOAK_LARGE").is_ok() {
        &large.url
    } else {
        &small.url
    };
    let relay = Relay::start(&bin(), &cloud.url(), url, &scale);
    let started = Instant::now();
    let mut samples = Vec::new();
    for k in (100..=cycles).step_by(100) {
        cloud.wait_for(k, Duration::from_secs(3_600));
        // Resident plus swapped out: the host may swap an idle relay.
        let rss = status_kb(relay.pid(), "VmRSS:") + status_kb(relay.pid(), "VmSwap:");
        let threads = status_kb(relay.pid(), "Threads:");
        eprintln!(
            "soak: {k} reconnects, {} served, RSS+swap {rss} kB, threads {threads}, {:?}",
            served.lock().unwrap(),
            started.elapsed()
        );
        samples.push(rss);
    }
    eprintln!("soak samples kB: {samples:?}");
}

#[cfg(target_os = "linux")]
fn frames_for_done(ws: &mut Ws, within: Duration) -> bool {
    let _ = ws.get_ref().set_read_timeout(Some(within));
    while let Ok(msg) = ws.read() {
        let Ok(v) = serde_json::from_str::<Value>(msg.to_text().unwrap_or("")) else {
            continue;
        };
        if v["type"] == "done" || v["type"] == "error" {
            return v["type"] == "done";
        }
    }
    false
}
