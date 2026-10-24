//! A launch driven by a script or agent never prompts, never installs, and
//! never starts a login; a person at a TTY is asked before anything installs.
#![cfg(unix)]

mod common;
use common::{serve, KEY};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A base URL nothing listens on: any request fails at once.
const DEAD: &str = "http://127.0.0.1:9/api";

fn home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("anyr-scripted-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("empty-path")).unwrap();
    dir
}

/// `anyr` with a fresh home and a PATH that has no agents and no `sh`, so a
/// wrongly started installer fails instead of touching the machine.
fn anyr(home: &PathBuf) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_anyr"));
    cmd.env_clear()
        .env("ANYROUTER_HOME", home)
        .env("HOME", home)
        .env("PATH", home.join("empty-path"))
        .env("ANYR_NO_UPDATE", "1")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb");
    cmd
}

fn wait(child: &mut Child, limit: Duration) -> Option<i32> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// A pseudo-terminal: the child gets the slave on stdin/stdout/stderr, the
/// test reads everything it prints from the master and can type into it.
struct Pty {
    master: File,
    output: Arc<Mutex<String>>,
}

fn spawn_on_pty(mut cmd: Command) -> (Child, Pty) {
    let (mut master, mut slave) = (0, 0);
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    assert_eq!(rc, 0, "openpty");
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    let master = unsafe { File::from_raw_fd(master) };
    cmd.stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave));
    let child = cmd.spawn().expect("spawn anyr on a pty");
    let output = Arc::new(Mutex::new(String::new()));
    let (mut reader, sink) = (master.try_clone().unwrap(), output.clone());
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    (child, Pty { master, output })
}

impl Pty {
    fn text(&self) -> String {
        self.output.lock().unwrap().clone()
    }

    fn wait_for(&self, needle: &str, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if self.text().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

#[test]
fn json_mode_on_a_tty_fails_with_the_key_hint_instead_of_logging_in() {
    let home = home("json-login");
    let mut cmd = anyr(&home);
    cmd.env("ANYR_OUTPUT", "json")
        .args(["codex", "--base-url", DEAD]);
    let (mut child, pty) = spawn_on_pty(cmd);
    let code = wait(&mut child, Duration::from_secs(15));
    let out = pty.text();
    assert_eq!(code, Some(4), "exit 4 (auth), not a login flow:\n{out}");
    assert!(out.contains("ANYROUTER_API_KEY"), "names the fix:\n{out}");
    assert!(!out.contains("Paste your AnyRouter API key"), "{out}");
}

#[test]
fn a_person_is_asked_before_a_missing_agent_is_installed() {
    let home = home("tty-install");
    let mut cmd = anyr(&home);
    cmd.env("ANYROUTER_API_KEY", KEY)
        .env("ANYR_NO_CATALOG", "1")
        .args([
            "claude",
            "--no-mcp",
            "--model",
            "owner/m",
            "--base-url",
            DEAD,
        ]);
    let (mut child, mut pty) = spawn_on_pty(cmd);
    let asked = pty.wait_for("Install it now?", Duration::from_secs(15));
    let _ = pty.master.write_all(b"n\n");
    let code = wait(&mut child, Duration::from_secs(15));
    let out = pty.text();
    assert!(asked, "asks y/N first:\n{out}");
    assert!(
        !out.contains("Installing"),
        "declined, so nothing runs:\n{out}"
    );
    assert_eq!(code, Some(3), "exit 3 (not found):\n{out}");
}

#[test]
fn a_scripted_launch_never_installs_and_names_the_install_command() {
    let home = home("pipe-install");
    let out = anyr(&home)
        .env("ANYROUTER_API_KEY", KEY)
        .env("ANYR_NO_CATALOG", "1")
        .args([
            "claude",
            "--no-mcp",
            "--model",
            "owner/m",
            "--base-url",
            DEAD,
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    assert!(!stderr.contains("Installing"), "{stderr}");
    let error: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(error.len(), 1, "one line:\n{stderr}");
    assert!(
        error[0].contains("npm install -g") && error[0].contains("--install"),
        "{stderr}"
    );
}

#[test]
fn pi_fetches_the_model_catalog_once() {
    let models = r#"{"data":[{"id":"owner/m","context_length":131072}]}"#;
    let (base, seen) = serve(vec![(200, models.into()), (200, models.into())]);
    let home = home("pi-once");
    let out = anyr(&home)
        .args(["pi", "--dry-run", "--model", "owner/m", "--key", KEY])
        .args(["--base-url", &base])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let paths: Vec<String> = seen.try_iter().map(|s| s.path).collect();
    assert_eq!(paths, vec!["/api/v1/models?privacy=0&sort=usage"]);
}
