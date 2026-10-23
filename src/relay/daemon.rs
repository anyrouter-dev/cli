//! Background relay: `start --detach`, `stop` and `logs`. The detached child
//! is this same binary running `relay start` with its own session, stdin
//! closed and output appended to `<config dir>/relay/relay.log`.

use std::io::{Read as _, Seek as _, SeekFrom};
use std::time::{Duration, Instant};

use super::state::{self, Conn, Paths, Running};
use super::view;

/// Internal env vars the parent hands the detached child. The credential
/// rides the environment, never argv, where `ps` would show it.
pub(super) const ENV_DETACHED: &str = "ANYR_RELAY_DETACHED";
pub(super) const ENV_SERVE: &str = "ANYR_RELAY_SERVE";

/// How long `start --detach` waits for the child to report connected.
const READY_WAIT: Duration = Duration::from_secs(10);
const STOP_WAIT: Duration = Duration::from_secs(10);

/// Everything the parent already resolved, so the child never needs a TTY.
pub(super) struct Handoff {
    pub args: Vec<String>,
    pub credential: String,
    pub api_key: Option<String>,
    /// Catalog ids of on-device models to serve (may be empty).
    pub serve: Vec<String>,
}

#[cfg(unix)]
pub(super) fn spawn(paths: &Paths, h: Handoff, bin: &str) -> Result<i32, String> {
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};

    std::fs::create_dir_all(&paths.dir)
        .map_err(|e| format!("could not create {}: {e}", paths.dir.display()))?;
    // Truncate per start so the log never grows without bound.
    let log = std::fs::File::create(&paths.log)
        .map_err(|e| format!("could not open {}: {e}", paths.log.display()))?;
    let exe =
        std::env::current_exe().map_err(|e| format!("could not find the anyr binary: {e}"))?;
    let mut cmd = Command::new(exe);
    cmd.args(&h.args)
        .env(ENV_DETACHED, "1")
        .env(ENV_SERVE, h.serve.join(","))
        .env(super::RELAY_TOKEN_ENV_VAR, &h.credential)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    if let Some(k) = &h.api_key {
        cmd.env("ANYROUTER_API_KEY", k);
    }
    // New session: no controlling terminal, so closing it does not stop us.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| view::error("could not start the background relay", &e.to_string(), ""))?;
    let pid = child.id();

    let deadline = Instant::now() + READY_WAIT;
    let mut connected = false;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(view::error(
                &format!("the background relay exited ({status})"),
                &format!("last log lines:\n{}", indent(&tail(&paths.log, 12))),
                &format!("`{bin} relay doctor`, or run it in the foreground: `{bin} relay start`"),
            ));
        }
        if let Running::Yes(s) = state::running(paths) {
            if s.pid == pid && s.conn == Conn::Connected {
                connected = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if connected {
        println!("Relay running in the background (pid {pid}), connected.");
    } else {
        println!("Relay started in the background (pid {pid}), still connecting.");
    }
    println!("  status  {bin} relay status");
    println!("  logs    {bin} relay logs --follow");
    println!("  stop    {bin} relay stop");
    Ok(0)
}

#[cfg(not(unix))]
pub(super) fn spawn(_paths: &Paths, _h: Handoff, bin: &str) -> Result<i32, String> {
    Err(view::error(
        "--detach is not supported on this platform",
        "background runs need a Unix session (macOS or Linux)",
        &format!("run `{bin} relay start` in its own terminal"),
    ))
}

/// `relay stop`: SIGTERM takes the same graceful path as Ctrl-C.
pub(super) fn stop(paths: &Paths, bin: &str) -> Result<i32, String> {
    let pid = match state::running(paths) {
        Running::No => {
            println!("Relay is not running.");
            return Ok(0);
        }
        Running::Yes(s) => s.pid,
        Running::Unresponsive(pid) => pid,
    };
    terminate(pid, bin)?;
    let deadline = Instant::now() + STOP_WAIT;
    while state::pid_alive(pid) {
        if Instant::now() > deadline {
            return Err(view::error(
                &format!(
                    "relay (pid {pid}) did not stop within {}s",
                    STOP_WAIT.as_secs()
                ),
                "it may be stuck on a slow local request",
                &format!("`kill -9 {pid}`"),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    state::clear_if_owned(paths, pid);
    println!("Stopped relay (pid {pid}).");
    Ok(0)
}

#[cfg(unix)]
fn terminate(pid: u32, _bin: &str) -> Result<(), String> {
    if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
        let e = std::io::Error::last_os_error();
        return Err(view::error(
            &format!("could not stop relay (pid {pid})"),
            &e.to_string(),
            &format!("`kill {pid}`"),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn terminate(pid: u32, bin: &str) -> Result<(), String> {
    Err(view::error(
        &format!("cannot signal relay (pid {pid}) on this platform"),
        "",
        &format!("press Ctrl-C in the terminal running `{bin} relay start`"),
    ))
}

/// Last `n` lines of `path`, or "" when it does not exist.
pub(super) fn tail(path: &std::path::Path, n: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    last_lines(&text, n)
}

pub(super) fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    let mut out = lines[start..].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `relay logs [--follow] [--lines N]`.
pub(super) fn logs(paths: &Paths, follow: bool, lines: usize, bin: &str) -> Result<i32, String> {
    if !paths.log.exists() {
        return Err(view::error(
            "no relay log yet",
            "only background runs write a log file",
            &format!("`{bin} relay start --detach`"),
        ));
    }
    print!("{}", tail(&paths.log, lines));
    if !follow {
        return Ok(0);
    }
    let mut file = std::fs::File::open(&paths.log).map_err(|e| e.to_string())?;
    let mut pos = file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let len = std::fs::metadata(&paths.log).map(|m| m.len()).unwrap_or(0);
        if len < pos {
            // A new `start --detach` truncated the log: follow the new run.
            file = std::fs::File::open(&paths.log).map_err(|e| e.to_string())?;
            pos = 0;
        }
        file.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let mut buf = String::new();
        let n = file.read_to_string(&mut buf).unwrap_or(0);
        if n > 0 {
            print!("{buf}");
            use std::io::Write as _;
            let _ = std::io::stdout().flush();
            pos += n as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_lines_keeps_the_tail() {
        assert_eq!(last_lines("a\nb\nc\n", 2), "b\nc\n");
        assert_eq!(last_lines("a\n", 5), "a\n");
        assert_eq!(last_lines("", 5), "");
    }
}
