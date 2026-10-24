//! `anyr chat` for scripts: `--json`, `--no-stream`, stdin, exit codes.

mod common;
use common::{run, serve};
use serde_json::Value;

const COMPLETION: &str = r#"{"id":"chatcmpl-1","model":"z-ai/glm-4.7-flash",
 "anyrouter_metadata":{"requestId":"req_abc"},
 "choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
 "usage":{"prompt_tokens":5,"completion_tokens":1}}"#;

#[test]
fn json_sends_one_non_stream_request_and_prints_one_object() {
    let (base, rx) = serve(vec![(200, COMPLETION.into())]);
    let (code, out, err) = run(
        &base,
        &[
            "chat",
            "Reply with ok",
            "--model",
            "z-ai/glm-4.7-flash",
            "--json",
        ],
        None,
    );
    assert_eq!(code, 0, "{err}");
    let seen = rx.recv().unwrap();
    assert_eq!(seen.path, "/api/v1/chat/completions");
    let body: Value = serde_json::from_str(&seen.body).unwrap();
    assert_eq!(body["stream"], false);
    assert_eq!(body["messages"][0]["content"], "Reply with ok");
    let v: Value = serde_json::from_str(&out).expect("stdout is one JSON object");
    // WHY: `id` is what `anyr generation <id>` takes, so a script can chain them.
    assert_eq!(v["id"], "req_abc");
    assert_eq!(v["content"], "ok");
    assert_eq!(v["finish_reason"], "stop");
    assert_eq!(v["usage"]["completion_tokens"], 1);
    assert_eq!(v["model"], "z-ai/glm-4.7-flash");
    // WHY: scripts read latency from `timing`; a non-stream reply has no
    // first-token time to report, so the key must be absent, not zero.
    assert!(v["timing"]["total_ms"].is_u64(), "{v}");
    assert!(v["timing"].get("ttft_ms").is_none(), "{v}");
}

#[test]
fn no_stream_prints_only_the_answer() {
    let (base, rx) = serve(vec![(200, COMPLETION.into())]);
    let (code, out, _) = run(&base, &["chat", "hi", "--no-stream"], None);
    assert_eq!(code, 0);
    assert_eq!(out, "ok\n");
    let body: Value = serde_json::from_str(&rx.recv().unwrap().body).unwrap();
    assert_eq!(body["stream"], false);
}

#[test]
fn a_dash_appends_piped_stdin_to_the_prompt() {
    let (base, rx) = serve(vec![(200, COMPLETION.into())]);
    let (code, _, err) = run(
        &base,
        &["chat", "Review this", "-", "--json"],
        Some("+ added line\n"),
    );
    assert_eq!(code, 0, "{err}");
    let body: Value = serde_json::from_str(&rx.recv().unwrap().body).unwrap();
    assert_eq!(body["messages"][0]["content"], "Review this + added line");
}

#[test]
fn piped_stdin_alone_is_the_prompt() {
    let (base, rx) = serve(vec![(200, COMPLETION.into())]);
    let (code, _, err) = run(&base, &["chat", "--json"], Some("from stdin\n"));
    assert_eq!(code, 0, "{err}");
    let body: Value = serde_json::from_str(&rx.recv().unwrap().body).unwrap();
    assert_eq!(body["messages"][0]["content"], "from stdin");
}

#[test]
fn failures_map_to_exit_codes_and_json_errors() {
    for (status, code, name) in [
        (401, 4, "auth"),
        (404, 3, "not_found"),
        (429, 5, "rate_limited"),
    ] {
        let (base, _rx) = serve(vec![(status, r#"{"error":{"message":"nope"}}"#.into())]);
        let (got, out, err) = run(&base, &["chat", "hi", "--json"], None);
        assert_eq!(got, code, "HTTP {status}: {err}");
        assert!(out.is_empty(), "{out}");
        let v: Value = serde_json::from_str(err.trim()).unwrap();
        assert_eq!(v["error"]["code"], name);
        assert_eq!(v["error"]["exit"], code);
        assert!(v["error"]["message"].as_str().unwrap().contains("nope"));
    }
}

#[test]
fn empty_input_is_a_usage_error_and_sends_nothing() {
    let (base, rx) = serve(vec![(200, COMPLETION.into())]);
    let (code, _, err) = run(&base, &["chat", "--json"], None);
    assert_eq!(code, 2, "{err}");
    assert!(rx
        .recv_timeout(std::time::Duration::from_millis(300))
        .is_err());
}

#[test]
fn a_server_error_that_echoes_the_key_is_masked() {
    // WHY: gateways echo the rejected Authorization value back; stderr lands in
    // CI logs and screenshots, so the raw key must never reach it.
    let key = "sk-ar-v1-abcdefghijklmnop1234567890";
    let body = format!(r#"{{"error":{{"message":"Invalid key {key} for request"}}}}"#);
    let (base, _rx) = serve(vec![(401, body)]);
    let (_, out, err) = run(&base, &["chat", "hi", "--no-stream"], None);
    assert!(!err.contains("abcdefghijklmnop1234567890"), "{err}");
    assert!(!out.contains("abcdefghijklmnop1234567890"), "{out}");
}

mod stream {
    use super::common::KEY;
    use serde_json::Value;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    const CHUNK_A: &str =
        r#"data: {"id":"gen-1","model":"cohere/x","choices":[{"delta":{"content":"AAA"}}]}"#;
    const CHUNK_B: &str =
        r#"data: {"id":"gen-1","model":"cohere/x","choices":[{"delta":{"content":"BBB"}}]}"#;
    const USAGE: &str = r#"data: {"id":"gen-1","model":"cohere/x","choices":[{"delta":{"content":""},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":11,"cost":0.0005}}"#;
    const META: &str = r#"data: {"id":"req_abcdef0123456789","model":"cohere/x","choices":[],"anyrouter_metadata":{"requestId":"req_abcdef0123456789"}}"#;

    /// Serves one SSE reply: sends the first chunk, then waits for `release`
    /// before sending the rest. The request body goes out on `seen`.
    fn serve_sse() -> (String, mpsc::Sender<()>, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/api", listener.local_addr().unwrap());
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (seen_tx, seen_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
            }
            let mut body = vec![0u8; len];
            r.read_exact(&mut body).unwrap();
            seen_tx.send(String::from_utf8(body).unwrap()).unwrap();
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nx-request-id: req_hdr\r\nConnection: close\r\n\r\n";
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(format!("{CHUNK_A}\n\n").as_bytes()).unwrap();
            s.flush().unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(10));
            let rest = format!("{CHUNK_B}\n\n{USAGE}\n\n{META}\n\ndata: [DONE]\n\n");
            let _ = s.write_all(rest.as_bytes());
        });
        (base, release_tx, seen_rx)
    }

    fn spawn(base: &str, stdout: Stdio, extra: &[&str]) -> Child {
        Command::new(env!("CARGO_BIN_EXE_anyr"))
            .env("ANYR_NO_UPDATE", "1")
            .env("NO_COLOR", "1")
            .env("ANYROUTER_API_KEY", KEY)
            .env(
                "ANYROUTER_HOME",
                std::env::temp_dir().join("anyr-common-test-empty"),
            )
            .args(["chat", "hi", "--model", "z-ai/glm-4.7-flash"])
            .args(extra)
            .args(["--base-url", base])
            .stdout(stdout)
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .unwrap()
    }

    #[test]
    fn deltas_reach_a_pipe_before_the_stream_ends_and_no_footer_is_added() {
        let (base, release, seen) = serve_sse();
        let mut child = spawn(&base, Stdio::piped(), &[]);
        let body: Value = serde_json::from_str(&seen.recv().unwrap()).unwrap();
        // WHY: usage only arrives on a stream when the request asks for it.
        assert_eq!(body["stream_options"]["include_usage"], true);
        let mut out = child.stdout.take().unwrap();
        let mut first = [0u8; 3];
        // The server is still holding the rest back, so this only returns if
        // the CLI flushed the first delta on its own.
        out.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"AAA");
        release.send(()).unwrap();
        let mut rest = String::new();
        out.read_to_string(&mut rest).unwrap();
        // WHY: a pipe is a script; extra lines would corrupt what it parses.
        assert_eq!(rest, "BBB\n");
        let mut err = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut err)
            .unwrap();
        assert!(child.wait().unwrap().success(), "{err}");
        assert!(!err.contains("tokens"), "{err}");
    }

    #[test]
    fn a_terminal_gets_one_footer_from_the_final_usage_chunk() {
        let (base, release, _seen) = serve_sse();
        release.send(()).unwrap();
        let (master, slave) = pty();
        let mut child = spawn(&base, Stdio::from(slave), &[]);
        let mut text = String::new();
        let mut f = std::fs::File::from(master);
        let _ = f.read_to_string(&mut text);
        assert!(child.wait().unwrap().success());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "AAABBB", "{text:?}");
        let footer = lines[1];
        for want in [
            "cohere/x",
            "7→11 tokens",
            "first token",
            "$0.0005",
            "req_abcdef01",
        ] {
            assert!(footer.contains(want), "{want} missing in {footer:?}");
        }
        assert_eq!(lines.len(), 2, "{text:?}");
    }

    #[test]
    fn quiet_on_a_terminal_prints_no_footer() {
        let (base, release, _seen) = serve_sse();
        release.send(()).unwrap();
        let (master, slave) = pty();
        let mut child = spawn(&base, Stdio::from(slave), &["--quiet"]);
        let mut text = String::new();
        let _ = std::fs::File::from(master).read_to_string(&mut text);
        assert!(child.wait().unwrap().success());
        assert_eq!(text.trim_end(), "AAABBB");
    }

    fn pty() -> (std::os::fd::OwnedFd, std::os::fd::OwnedFd) {
        use std::os::fd::FromRawFd;
        let (mut m, mut s) = (0, 0);
        let rc = unsafe {
            libc::openpty(
                &mut m,
                &mut s,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(rc, 0);
        unsafe {
            (
                std::os::fd::OwnedFd::from_raw_fd(m),
                std::os::fd::OwnedFd::from_raw_fd(s),
            )
        }
    }
}
