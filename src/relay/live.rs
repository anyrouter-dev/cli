//! Runtime control for a running relay: shared counters, the stop/rescan
//! flags that signals and keys flip, output routing, the state-file ticker,
//! and the one-screen live view at a terminal.

use std::collections::VecDeque;
use std::io::IsTerminal as _;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::state::{self, Conn, Paths, Snapshot};
use super::view::{self, Paint};

// ---------------------------------------------------------------------------
// Stop / rescan flags
// ---------------------------------------------------------------------------

/// 0 = keep running, `STOP_INT` = Ctrl-C, `STOP_QUIT` = SIGTERM or `q`.
static STOP: AtomicU8 = AtomicU8::new(0);
pub(super) const STOP_INT: u8 = 1;
pub(super) const STOP_QUIT: u8 = 2;
static RESCAN: AtomicBool = AtomicBool::new(false);

pub(super) fn stop_requested() -> u8 {
    STOP.load(Ordering::SeqCst)
}

pub(super) fn request_stop(code: u8) {
    STOP.store(code, Ordering::SeqCst);
}

pub(super) fn take_rescan() -> bool {
    RESCAN.swap(false, Ordering::SeqCst)
}

#[cfg(unix)]
extern "C" fn on_signal(sig: libc::c_int) {
    let code = if sig == libc::SIGINT {
        STOP_INT
    } else {
        STOP_QUIT
    };
    // A second Ctrl-C while shutting down exits at once (clig.dev). The
    // normal restore path is skipped by `_exit`, so undo raw mode and the
    // alternate screen here with async-signal-safe calls only.
    if STOP.swap(code, Ordering::SeqCst) != 0 {
        unsafe {
            if let Some(t) = SAVED_TERMIOS.get() {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t);
            }
            if LIVE.load(Ordering::Relaxed) {
                let b = LEAVE_SCREEN.as_bytes();
                libc::write(libc::STDOUT_FILENO, b.as_ptr().cast(), b.len());
            }
            libc::_exit(130)
        };
    }
}

/// Terminal settings from before raw mode, for the signal path.
#[cfg(unix)]
static SAVED_TERMIOS: std::sync::OnceLock<libc::termios> = std::sync::OnceLock::new();

/// SIGINT and SIGTERM ask for a graceful stop. A detached relay also ignores
/// SIGHUP so closing the terminal that launched it does not kill it.
pub(super) fn install_signal_handlers(detached: bool) {
    #[cfg(unix)]
    unsafe {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
        if detached {
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
    }
    #[cfg(not(unix))]
    let _ = detached;
}

/// Sleep up to `total`, waking early for a stop or rescan request.
pub(super) fn interruptible_sleep(total: Duration) {
    let step = Duration::from_millis(100);
    let mut left = total;
    while !left.is_zero() {
        if stop_requested() != 0 || RESCAN.load(Ordering::SeqCst) {
            return;
        }
        let d = left.min(step);
        std::thread::sleep(d);
        left -= d;
    }
}

// ---------------------------------------------------------------------------
// Output routing: plain lines, or events inside the live screen
// ---------------------------------------------------------------------------

static LIVE: AtomicBool = AtomicBool::new(false);
static QUIET: AtomicBool = AtomicBool::new(false);
static EVENTS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const MAX_EVENTS: usize = 5;

pub(super) fn set_quiet(on: bool) {
    QUIET.store(on, Ordering::Relaxed);
}

/// Informational line. While the live screen owns the terminal a raw
/// `eprintln` would tear the redraw, so it becomes an on-screen event.
pub(super) fn log(msg: &str) {
    // One event per line (screen and log file alike): fold multi-line errors.
    let msg = msg.lines().map(str::trim).collect::<Vec<_>>().join(" ");
    if LIVE.load(Ordering::Relaxed) {
        let mut ev = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
        ev.push_back(format!(
            "{} {msg}",
            &view::utc_stamp(state::now_secs())[11..19]
        ));
        while ev.len() > MAX_EVENTS {
            ev.pop_front();
        }
        return;
    }
    if QUIET.load(Ordering::Relaxed) {
        return;
    }
    if std::io::stderr().is_terminal() {
        eprintln!("{msg}");
    } else {
        // Background log files and pipes get a timestamp per line.
        eprintln!("{} {msg}", view::utc_stamp(state::now_secs()));
    }
}

fn events() -> Vec<String> {
    EVENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Shared counters
// ---------------------------------------------------------------------------

/// Everything the status screen and state file show. Written by the socket
/// loop, read by the ticker.
pub(super) struct Stats {
    pub requests: AtomicU64,
    pub in_flight: AtomicU64,
    pub errors: AtomicU64,
    pub tokens: AtomicU64,
    pub tokens_seen: AtomicBool,
    pub pool: AtomicBool,
    pub conn: Mutex<Conn>,
    pub last_error: Mutex<Option<String>>,
    pub models: Mutex<Vec<String>>,
    pub endpoints: Mutex<Vec<(String, String)>>,
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            requests: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            tokens: AtomicU64::new(0),
            tokens_seen: AtomicBool::new(false),
            pool: AtomicBool::new(false),
            conn: Mutex::new(Conn::Connecting),
            last_error: Mutex::new(None),
            models: Mutex::new(Vec::new()),
            endpoints: Mutex::new(Vec::new()),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Stats {
    pub fn set_conn(&self, conn: Conn) {
        *lock(&self.conn) = conn;
    }

    pub fn set_error(&self, err: Option<String>) {
        *lock(&self.last_error) = err;
    }

    pub fn set_router(&self, models: Vec<String>, endpoints: Vec<(String, String)>) {
        *lock(&self.models) = models;
        *lock(&self.endpoints) = endpoints;
    }

    pub fn add_tokens(&self, n: u64) {
        self.tokens.fetch_add(n, Ordering::Relaxed);
        self.tokens_seen.store(true, Ordering::Relaxed);
    }

    pub fn finish_request(&self, failed: bool) {
        // Saturating decrement. A CAS loop rather than `fetch_update`, which
        // newer toolchains deprecate in favor of a method older ones lack.
        let mut n = self.in_flight.load(Ordering::Relaxed);
        while n > 0 {
            match self.in_flight.compare_exchange_weak(
                n,
                n - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(cur) => n = cur,
            }
        }
        if failed {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self, meta: &Meta, now: u64) -> Snapshot {
        Snapshot {
            pid: meta.pid,
            started_at: meta.started_at,
            updated_at: now,
            detached: meta.detached,
            device: meta.device.clone(),
            conn: lock(&self.conn).clone(),
            last_error: lock(&self.last_error).clone(),
            pool: self.pool.load(Ordering::Relaxed),
            models: lock(&self.models).clone(),
            endpoints: lock(&self.endpoints).clone(),
            requests: self.requests.load(Ordering::Relaxed),
            in_flight: self.in_flight.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            tokens: self
                .tokens_seen
                .load(Ordering::Relaxed)
                .then(|| self.tokens.load(Ordering::Relaxed)),
        }
    }
}

/// Fixed facts about this relay process.
#[derive(Clone)]
pub(super) struct Meta {
    pub pid: u32,
    pub started_at: u64,
    pub detached: bool,
    pub device: String,
}

/// Largest `"total_tokens": N` in `text`. Responses report usage once
/// (non-stream body, or the final SSE chunk), so the max is that request's
/// total.
pub(super) fn total_tokens(text: &str) -> Option<u64> {
    const KEY: &str = "\"total_tokens\"";
    let mut best = None;
    let mut rest = text;
    while let Some(i) = rest.find(KEY) {
        rest = &rest[i + KEY.len()..];
        let after = rest.trim_start().strip_prefix(':').map(str::trim_start);
        if let Some(num) = after {
            let digits: String = num.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = digits.parse::<u64>() {
                best = Some(best.map_or(n, |b: u64| b.max(n)));
            }
        }
    }
    best
}

// ---------------------------------------------------------------------------
// Ticker + live screen
// ---------------------------------------------------------------------------

/// Whether `start` should take over the terminal with the live screen.
pub(super) fn wants_live_screen(quiet: bool, detached: bool, env_term: Option<&str>) -> bool {
    !quiet
        && !detached
        && env_term != Some("dumb")
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
}

const ENTER_SCREEN: &str = "\x1b[?1049h\x1b[?25l";
const LEAVE_SCREEN: &str = "\x1b[?25h\x1b[?1049l";

fn leave_screen() {
    use std::io::Write as _;
    let _ = crossterm::terminal::disable_raw_mode();
    let mut out = std::io::stdout();
    let _ = out.write_all(LEAVE_SCREEN.as_bytes());
    let _ = out.flush();
}

/// Background work for one relay run: refresh the state file every second,
/// redraw the live screen, and (live only) read keys.
pub(super) struct Ticker {
    done: Arc<AtomicBool>,
    handles: Vec<std::thread::JoinHandle<()>>,
    live: bool,
}

/// What the `p` key needs to persist and push the pool change.
pub(super) type PoolToggle = Box<dyn Fn(bool) + Send>;

impl Ticker {
    pub fn start(
        stats: Arc<Stats>,
        meta: Meta,
        paths: Paths,
        live: bool,
        follow_pool_pref: bool,
        on_pool: PoolToggle,
    ) -> Ticker {
        let done = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        if live {
            LIVE.store(true, Ordering::Relaxed);
            let prev = std::panic::take_hook();
            // Release builds abort on panic, so no Drop runs: restore the
            // terminal from the hook instead.
            std::panic::set_hook(Box::new(move |info| {
                leave_screen();
                prev(info);
            }));
            #[cfg(unix)]
            unsafe {
                let mut t: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(libc::STDIN_FILENO, &mut t) == 0 {
                    let _ = SAVED_TERMIOS.set(t);
                }
            }
            let _ = crossterm::terminal::enable_raw_mode();
            print!("{ENTER_SCREEN}");
            let (stats_k, done_k) = (Arc::clone(&stats), Arc::clone(&done));
            handles.push(std::thread::spawn(move || {
                read_keys(&stats_k, &done_k, &on_pool)
            }));
        }
        let done_t = Arc::clone(&done);
        handles.push(std::thread::spawn(move || {
            let mut tick: u64 = 0;
            loop {
                let finished = done_t.load(Ordering::SeqCst);
                // `relay pool on|off` from another terminal shows up here.
                if follow_pool_pref && tick.is_multiple_of(4) {
                    if let Some(on) = state::load_prefs(&paths).pool {
                        stats.pool.store(on, Ordering::Relaxed);
                    }
                }
                let snap = stats.snapshot(&meta, state::now_secs());
                if tick.is_multiple_of(2) || finished {
                    let _ = state::write_snapshot(&paths, &snap);
                }
                if live && !finished {
                    draw(&snap);
                }
                if finished {
                    return;
                }
                tick += 1;
                std::thread::sleep(Duration::from_millis(500));
            }
        }));
        Ticker {
            done,
            handles,
            live,
        }
    }

    /// Stop the threads, write the final snapshot and give the terminal back.
    pub fn finish(self) {
        self.done.store(true, Ordering::SeqCst);
        for h in self.handles {
            let _ = h.join();
        }
        if self.live {
            LIVE.store(false, Ordering::Relaxed);
            leave_screen();
        }
    }
}

fn draw(s: &Snapshot) {
    use std::io::Write as _;
    let p = Paint(crate::term::color_enabled());
    let body =
        view::render_running(s, state::now_secs(), p) + &view::render_live_footer(&events(), p);
    // Raw mode: every line needs an explicit carriage return.
    let frame = format!("\x1b[H\x1b[J{}", body.replace('\n', "\r\n"));
    let mut out = std::io::stdout();
    let _ = out.write_all(frame.as_bytes());
    let _ = out.flush();
}

fn read_keys(stats: &Stats, done: &AtomicBool, on_pool: &PoolToggle) {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    while !done.load(Ordering::SeqCst) {
        if !event::poll(Duration::from_millis(200)).unwrap_or(false) {
            continue;
        }
        let Ok(Event::Key(k)) = event::read() else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        match k.code {
            // Raw mode turns Ctrl-C into a key, not SIGINT.
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if stop_requested() != 0 {
                    leave_screen();
                    std::process::exit(130);
                }
                request_stop(STOP_INT);
            }
            KeyCode::Char('q') | KeyCode::Esc => request_stop(STOP_QUIT),
            KeyCode::Char('r') => {
                RESCAN.store(true, Ordering::SeqCst);
                log("rescanning local servers");
            }
            KeyCode::Char('p') => {
                let on = !stats.pool.load(Ordering::Relaxed);
                stats.pool.store(on, Ordering::Relaxed);
                on_pool(on);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counter_reads_usage_from_bodies_and_sse() {
        let body =
            r#"{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":7,"total_tokens":12}}"#;
        assert_eq!(total_tokens(body), Some(12));
        let sse = "data: {\"usage\": {\"total_tokens\" : 40}}\n\ndata: [DONE]\n";
        assert_eq!(total_tokens(sse), Some(40));
        // WHY: fm serve may never report usage; that must read as unknown,
        // not as zero tokens.
        assert_eq!(total_tokens(r#"{"choices":[]}"#), None);
    }

    #[test]
    fn stats_snapshot_counts_and_reports_unknown_tokens() {
        let s = Stats::default();
        let meta = Meta {
            pid: 1,
            started_at: 0,
            detached: false,
            device: "d".into(),
        };
        assert_eq!(s.snapshot(&meta, 5).tokens, None);
        s.requests.fetch_add(2, Ordering::Relaxed);
        s.in_flight.fetch_add(2, Ordering::Relaxed);
        s.finish_request(false);
        s.finish_request(true);
        s.finish_request(true); // never underflows
        s.add_tokens(30);
        let snap = s.snapshot(&meta, 5);
        assert_eq!((snap.requests, snap.in_flight, snap.errors), (2, 0, 2));
        assert_eq!(snap.tokens, Some(30));
    }

    #[test]
    fn live_screen_never_runs_quiet_detached_or_on_a_dumb_terminal() {
        assert!(!wants_live_screen(true, false, None));
        assert!(!wants_live_screen(false, true, None));
        assert!(!wants_live_screen(false, false, Some("dumb")));
    }
}
