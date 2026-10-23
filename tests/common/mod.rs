//! Shared scripted-HTTP-server helpers for integration tests.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;

pub const KEY: &str = "sk-ar-v1-testkey0123456789";

/// One request as the server saw it.
#[derive(Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub auth: String,
    pub body: String,
}

/// Serve `replies` in order, one per connection.
pub fn serve(replies: Vec<(u16, String)>) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/api", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (status, body) in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let (mut auth, mut len) = (String::new(), 0usize);
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
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
            let mut req_body = vec![0u8; len];
            reader.read_exact(&mut req_body).unwrap();
            let mut parts = line.split_whitespace();
            let seen = Seen {
                method: parts.next().unwrap_or("").into(),
                path: parts.next().unwrap_or("").into(),
                auth,
                body: String::from_utf8_lossy(&req_body).into_owned(),
            };
            let resp = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).unwrap();
            tx.send(seen).unwrap();
        }
    });
    (base, rx)
}

/// Run `anyr <args> --base-url <base>` with a throwaway home; returns
/// (exit code, stdout, stderr). stdin is `input` or /dev/null.
pub fn run(base: &str, args: &[&str], input: Option<&str>) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_anyr"));
    cmd.env("ANYR_NO_UPDATE", "1")
        .env("NO_COLOR", "1")
        .env("ANYROUTER_API_KEY", KEY)
        .env_remove("ANYROUTER_MANAGEMENT_KEY")
        .env(
            "ANYROUTER_HOME",
            std::env::temp_dir().join("anyr-common-test-empty"),
        )
        .args(args)
        .args(["--base-url", base])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = cmd.spawn().expect("spawn anyr");
    if let Some(i) = input {
        let _ = child.stdin.take().unwrap().write_all(i.as_bytes());
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}
