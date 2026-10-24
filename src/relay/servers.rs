//! Local model servers the relay can start itself, so `anyr relay start`
//! needs no second terminal.
//!
//! [`SERVERS`] is the one table of known on-device servers: how to tell one
//! is up, how to start it, and which model it serves. A server the user
//! already runs is left alone ("external"); one the relay started is
//! "managed": restarted with backoff if it dies, stopped when the relay exits.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::OnDeviceModel;

/// How a server is started and stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Launch {
    /// A CLI run as the relay's own child process; stopped by killing it.
    Child {
        program: &'static str,
        args: &'static [&'static str],
    },
    /// A macOS app in /Applications, started with `open -g -a <app>` and
    /// stopped by asking it to quit.
    MacApp { app: &'static str },
}

/// How to tell the server answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Health {
    /// Any HTTP response from this URL.
    Get(&'static str),
    /// A real decision POSTed to `<base>/decisions` returns `{answers}`.
    Decider(&'static str),
}

/// One known on-device server.
#[derive(Debug)]
pub(super) struct ServerSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// The base URL discovery finds it at.
    pub base: &'static str,
    pub health: Health,
    pub launch: Launch,
    /// The catalog model it serves (its `id()` and hello `local_id()`).
    pub model: OnDeviceModel,
    /// How long start waits for it to answer before moving on.
    pub ready_timeout: Duration,
}

/// Known on-device servers. Adding one is one entry here.
pub(super) static SERVERS: [ServerSpec; 2] = [
    ServerSpec {
        id: "fm-serve",
        label: "fm serve (Apple Foundation Models)",
        base: super::FM_SERVE_TARGET,
        health: Health::Get(super::FM_SERVE_HEALTH_URL),
        launch: Launch::Child {
            program: "fm",
            args: &["serve"],
        },
        model: OnDeviceModel::AppleFoundation,
        ready_timeout: Duration::from_secs(20),
    },
    ServerSpec {
        id: "decider",
        label: "Perplexity Decider",
        base: super::DECIDER_TARGET,
        health: Health::Decider(super::DECIDER_TARGET),
        // Perplexity's docs: the app is "Decider" in Applications, opening it
        // starts the local API (menu bar only), Quit stops it. No CLI.
        launch: Launch::MacApp { app: "Decider" },
        model: OnDeviceModel::DeciderFlash,
        // The first launch downloads ~5.8 GB before it answers; start does
        // not wait for that, supervision keeps watching.
        ready_timeout: Duration::from_secs(60),
    },
];

pub(super) fn spec_for(model: OnDeviceModel) -> Option<&'static ServerSpec> {
    SERVERS.iter().find(|s| s.model == model)
}

/// Everything the supervisor does to the outside world, behind a trait so
/// tests run off-Mac with no processes or sockets.
pub(super) trait Host: Send {
    /// The server answers its health check right now.
    fn healthy(&self, spec: &ServerSpec) -> bool;
    /// A process for this server exists (started by anyone).
    fn running(&self, spec: &ServerSpec) -> bool;
    /// It can be launched on this machine.
    fn installed(&self, spec: &ServerSpec) -> bool;
    fn launch(&mut self, spec: &ServerSpec) -> Result<(), String>;
    /// The copy this host launched is gone.
    fn exited(&mut self, spec: &ServerSpec) -> bool;
    fn stop(&mut self, spec: &ServerSpec);
    fn now(&self) -> Instant;
    fn sleep(&self, d: Duration);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    /// Already running when the relay started; never stopped by us.
    External,
    /// Started by the relay; stopped on exit.
    Managed,
    /// Not served: not installed, `--no-launch`, or the launch failed.
    Off,
}

impl Status {
    pub(super) fn tag(self) -> Option<&'static str> {
        match self {
            Status::External => Some("external"),
            Status::Managed => Some("managed"),
            Status::Off => None,
        }
    }
}

struct Entry {
    spec: &'static ServerSpec,
    status: Status,
    /// Restarts since it was last stable.
    restarts: u32,
    /// Set while a crashed managed server waits for its restart.
    restart_at: Option<Instant>,
    launched_at: Instant,
    healthy: bool,
}

/// First restart waits this long, doubling per failure up to the cap.
pub(super) const RESTART_BASE: Duration = Duration::from_secs(1);
pub(super) const RESTART_MAX: Duration = Duration::from_secs(60);
/// Healthy this long after a launch resets the backoff.
const STABLE_AFTER: Duration = Duration::from_secs(60);
const READY_POLL: Duration = Duration::from_millis(500);

pub(super) fn restart_delay(restarts: u32) -> Duration {
    RESTART_BASE
        .saturating_mul(1u32 << restarts.min(16))
        .min(RESTART_MAX)
}

pub(super) struct Supervisor {
    host: Box<dyn Host>,
    entries: Vec<Entry>,
    log: Box<dyn Fn(&str) + Send>,
}

impl Supervisor {
    /// Find or start each server in `specs`. External ones are only noted;
    /// missing ones are launched unless `no_launch`, then given up to their
    /// `ready_timeout` to answer.
    pub(super) fn start(
        specs: Vec<&'static ServerSpec>,
        no_launch: bool,
        host: Box<dyn Host>,
        log: Box<dyn Fn(&str) + Send>,
    ) -> Self {
        let now = host.now();
        let mut sup = Supervisor {
            host,
            entries: Vec::new(),
            log,
        };
        for spec in specs {
            let healthy = sup.host.healthy(spec);
            let status = if healthy || sup.host.running(spec) {
                (sup.log)(&format!("found {} running (external)", spec.label));
                Status::External
            } else if no_launch {
                (sup.log)(&format!(
                    "{} is not running; not starting it (--no-launch)",
                    spec.label
                ));
                Status::Off
            } else if !sup.host.installed(spec) {
                (sup.log)(&format!("{} is not running and not installed", spec.label));
                Status::Off
            } else {
                match sup.host.launch(spec) {
                    Ok(()) => {
                        (sup.log)(&format!("started {} (managed)", spec.label));
                        Status::Managed
                    }
                    Err(e) => {
                        (sup.log)(&format!("could not start {}: {e}", spec.label));
                        Status::Off
                    }
                }
            };
            sup.entries.push(Entry {
                spec,
                status,
                restarts: 0,
                restart_at: None,
                launched_at: now,
                healthy,
            });
        }
        sup.wait_ready();
        sup
    }

    /// Wait for every managed server to answer, each up to its timeout.
    fn wait_ready(&mut self) {
        let start = self.host.now();
        loop {
            let mut waiting = false;
            for e in &mut self.entries {
                if e.status != Status::Managed || e.healthy {
                    continue;
                }
                e.healthy = self.host.healthy(e.spec);
                if !e.healthy && self.host.now() < start + e.spec.ready_timeout {
                    waiting = true;
                }
            }
            if !waiting || super::live::stop_requested() != 0 {
                break;
            }
            self.host.sleep(READY_POLL);
        }
        for e in &self.entries {
            if e.status == Status::Managed && !e.healthy {
                (self.log)(&format!(
                    "{} is still starting after {}s; serving it once it answers",
                    e.spec.label,
                    e.spec.ready_timeout.as_secs()
                ));
            }
        }
    }

    /// One supervision pass: restart managed servers that died, with
    /// backoff. Returns true when a managed server came up (or back), so the
    /// caller rescans endpoints.
    pub(super) fn tick(&mut self) -> bool {
        let now = self.host.now();
        let mut came_up = false;
        for e in &mut self.entries {
            if e.status != Status::Managed {
                continue;
            }
            if let Some(at) = e.restart_at {
                if now < at {
                    continue;
                }
                e.restart_at = None;
                match self.host.launch(e.spec) {
                    Ok(()) => {
                        (self.log)(&format!(
                            "restarted {} (attempt {})",
                            e.spec.label, e.restarts
                        ));
                        e.launched_at = now;
                        e.healthy = false;
                    }
                    Err(err) => {
                        let delay = restart_delay(e.restarts);
                        e.restarts += 1;
                        e.restart_at = Some(now + delay);
                        (self.log)(&format!(
                            "could not restart {}: {err}; retrying in {}s",
                            e.spec.label,
                            delay.as_secs()
                        ));
                    }
                }
                continue;
            }
            if self.host.exited(e.spec) {
                let delay = restart_delay(e.restarts);
                e.restarts += 1;
                e.restart_at = Some(now + delay);
                e.healthy = false;
                (self.log)(&format!(
                    "{} exited; restarting in {}s",
                    e.spec.label,
                    delay.as_secs()
                ));
                continue;
            }
            let healthy = self.host.healthy(e.spec);
            if healthy && !e.healthy {
                came_up = true;
            }
            e.healthy = healthy;
            if healthy && e.restarts > 0 && now.duration_since(e.launched_at) >= STABLE_AFTER {
                e.restarts = 0;
            }
        }
        came_up
    }

    /// Stop every server the relay started. External ones keep running.
    pub(super) fn stop_all(&mut self) {
        for e in &mut self.entries {
            if e.status == Status::Managed {
                self.host.stop(e.spec);
                (self.log)(&format!("stopped {}", e.spec.label));
                e.status = Status::Off;
            }
        }
    }

    /// `(managed)` / `(external)` per base URL, for the Endpoints line.
    pub(super) fn tags(&self) -> BTreeMap<&'static str, &'static str> {
        self.entries
            .iter()
            .filter_map(|e| e.status.tag().map(|t| (e.spec.base, t)))
            .collect()
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop_all();
    }
}

// ---------------------------------------------------------------------------
// The real host: processes and HTTP on this Mac
// ---------------------------------------------------------------------------

#[derive(Default)]
pub(super) struct SystemHost {
    children: BTreeMap<&'static str, std::process::Child>,
}

/// `pgrep -f` pattern for a child command: `fm serve`, by any path.
fn child_pattern(program: &str, args: &[&str]) -> String {
    let mut s = format!("(^|/){program}");
    for a in args {
        s.push(' ');
        s.push_str(a);
    }
    s.push_str("( |$)");
    s
}

fn quiet_status(cmd: &mut std::process::Command) -> bool {
    use std::process::Stdio;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

impl Host for SystemHost {
    fn healthy(&self, spec: &ServerSpec) -> bool {
        match spec.health {
            Health::Get(url) => super::http_probe_listening(url),
            Health::Decider(base) => {
                let ok = super::probe_decider_uncached(base);
                if !ok {
                    super::forget_decider(base);
                }
                ok
            }
        }
    }

    fn running(&self, spec: &ServerSpec) -> bool {
        let mut cmd = std::process::Command::new("pgrep");
        match spec.launch {
            Launch::Child { program, args } => cmd.args(["-f", &child_pattern(program, args)]),
            Launch::MacApp { app } => cmd.args(["-x", app]),
        };
        quiet_status(&mut cmd)
    }

    fn installed(&self, spec: &ServerSpec) -> bool {
        if !cfg!(target_os = "macos") {
            return false;
        }
        match spec.launch {
            Launch::Child { program, .. } => crate::install::find_on_path(program).is_some(),
            Launch::MacApp { app } => {
                let bundle = format!("{app}.app");
                let mut dirs = vec![std::path::PathBuf::from("/Applications")];
                if let Some(home) = std::env::var_os("HOME") {
                    dirs.push(std::path::Path::new(&home).join("Applications"));
                }
                dirs.iter().any(|d| d.join(&bundle).is_dir())
            }
        }
    }

    fn launch(&mut self, spec: &ServerSpec) -> Result<(), String> {
        use std::process::{Command, Stdio};
        match spec.launch {
            Launch::Child { program, args } => {
                let mut cmd = Command::new(program);
                // Output would draw over the live screen.
                cmd.args(args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                // Own process group: a terminal Ctrl-C reaches the relay,
                // which then stops this child itself.
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt as _;
                    cmd.process_group(0);
                }
                let child = cmd.spawn().map_err(|e| format!("`{program}`: {e}"))?;
                super::live::track_child(child.id());
                self.children.insert(spec.id, child);
                Ok(())
            }
            Launch::MacApp { app } => {
                if quiet_status(Command::new("open").args(["-g", "-a", app])) {
                    Ok(())
                } else {
                    Err(format!("`open -g -a {app}` failed"))
                }
            }
        }
    }

    fn exited(&mut self, spec: &ServerSpec) -> bool {
        match spec.launch {
            Launch::Child { .. } => match self.children.get_mut(spec.id) {
                Some(child) => match child.try_wait() {
                    Ok(None) => false,
                    _ => {
                        super::live::untrack_child(child.id());
                        self.children.remove(spec.id);
                        true
                    }
                },
                None => true,
            },
            Launch::MacApp { .. } => !self.running(spec),
        }
    }

    fn stop(&mut self, spec: &ServerSpec) {
        match spec.launch {
            Launch::Child { .. } => {
                if let Some(mut child) = self.children.remove(spec.id) {
                    super::live::untrack_child(child.id());
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            Launch::MacApp { app } => {
                quiet_status(
                    std::process::Command::new("osascript")
                        .args(["-e", &format!("quit app \"{app}\"")]),
                );
            }
        }
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, d: Duration) {
        super::live::interruptible_sleep(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Scripted world: which servers answer, run, are installed; records
    /// every launch and stop. Time is virtual.
    #[derive(Default)]
    struct World {
        healthy: BTreeMap<&'static str, bool>,
        running: BTreeMap<&'static str, bool>,
        installed: BTreeMap<&'static str, bool>,
        /// A launch makes the server healthy (fm-style) unless listed here.
        slow: Vec<&'static str>,
        fail_launch: Vec<&'static str>,
        launches: Vec<&'static str>,
        stops: Vec<&'static str>,
        clock: Duration,
    }

    #[derive(Clone)]
    struct Mock(Arc<Mutex<World>>, Instant);

    impl Mock {
        fn new(w: World) -> Self {
            Mock(Arc::new(Mutex::new(w)), Instant::now())
        }
        fn w(&self) -> std::sync::MutexGuard<'_, World> {
            self.0.lock().unwrap()
        }
        fn crash(&self, id: &'static str) {
            let mut w = self.w();
            w.healthy.insert(id, false);
            w.running.insert(id, false);
        }
        fn advance(&self, d: Duration) {
            self.w().clock += d;
        }
    }

    impl Host for Mock {
        fn healthy(&self, s: &ServerSpec) -> bool {
            self.w().healthy.get(s.id).copied().unwrap_or(false)
        }
        fn running(&self, s: &ServerSpec) -> bool {
            self.w().running.get(s.id).copied().unwrap_or(false)
        }
        fn installed(&self, s: &ServerSpec) -> bool {
            self.w().installed.get(s.id).copied().unwrap_or(false)
        }
        fn launch(&mut self, s: &ServerSpec) -> Result<(), String> {
            let mut w = self.w();
            w.launches.push(s.id);
            if w.fail_launch.contains(&s.id) {
                return Err("boom".into());
            }
            w.running.insert(s.id, true);
            if !w.slow.contains(&s.id) {
                w.healthy.insert(s.id, true);
            }
            Ok(())
        }
        fn exited(&mut self, s: &ServerSpec) -> bool {
            !self.running(s)
        }
        fn stop(&mut self, s: &ServerSpec) {
            let mut w = self.w();
            w.stops.push(s.id);
            w.running.insert(s.id, false);
            w.healthy.insert(s.id, false);
        }
        fn now(&self) -> Instant {
            self.1 + self.w().clock
        }
        fn sleep(&self, d: Duration) {
            self.advance(d);
        }
    }

    fn fm() -> &'static ServerSpec {
        spec_for(OnDeviceModel::AppleFoundation).unwrap()
    }
    fn decider() -> &'static ServerSpec {
        spec_for(OnDeviceModel::DeciderFlash).unwrap()
    }

    fn installed_all() -> World {
        World {
            installed: [("fm-serve", true), ("decider", true)].into(),
            ..World::default()
        }
    }

    fn start(m: &Mock, no_launch: bool) -> (Supervisor, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let sup = Supervisor::start(
            vec![fm(), decider()],
            no_launch,
            Box::new(m.clone()),
            Box::new(move |l| sink.lock().unwrap().push(l.to_string())),
        );
        (sup, lines)
    }

    #[test]
    fn every_on_device_model_has_one_server_entry() {
        // WHY: the table is the only place a launchable server is declared;
        // a model without an entry would silently never auto-start.
        for m in OnDeviceModel::ALL {
            assert_eq!(SERVERS.iter().filter(|s| s.model == m).count(), 1, "{m:?}");
            assert_eq!(spec_for(m).unwrap().base, m.base());
        }
        let ids: std::collections::BTreeSet<_> = SERVERS.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), SERVERS.len());
    }

    #[test]
    fn launch_commands_match_the_documented_ones() {
        assert_eq!(
            fm().launch,
            Launch::Child {
                program: "fm",
                args: &["serve"]
            }
        );
        assert_eq!(decider().launch, Launch::MacApp { app: "Decider" });
        assert_eq!(
            decider().health,
            Health::Decider("http://127.0.0.1:8080/v1")
        );
        assert_eq!(fm().health, Health::Get("http://127.0.0.1:1976/health"));
    }

    #[test]
    fn missing_installed_servers_are_launched_and_managed() {
        let m = Mock::new(installed_all());
        let (sup, lines) = start(&m, false);
        assert_eq!(m.w().launches, vec!["fm-serve", "decider"]);
        let tags = sup.tags();
        assert_eq!(tags[fm().base], "managed");
        assert_eq!(tags[decider().base], "managed");
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == "started Perplexity Decider (managed)"));
    }

    #[test]
    fn a_server_already_answering_is_external_and_never_launched() {
        let mut w = installed_all();
        w.healthy.insert("fm-serve", true);
        let m = Mock::new(w);
        let (sup, lines) = start(&m, false);
        assert_eq!(m.w().launches, vec!["decider"], "no second fm serve");
        assert_eq!(sup.tags()[fm().base], "external");
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == "found fm serve (Apple Foundation Models) running (external)"));
    }

    #[test]
    fn a_running_but_still_loading_app_is_external_not_relaunched() {
        // WHY: Decider downloads its model before it answers. A user-opened
        // copy that is still loading must not be launched again, or quit on
        // exit as if the relay owned it.
        let mut w = installed_all();
        w.running.insert("decider", true);
        let m = Mock::new(w);
        let (mut sup, _) = start(&m, false);
        assert!(!m.w().launches.contains(&"decider"));
        assert_eq!(sup.tags()[decider().base], "external");
        sup.stop_all();
        assert!(!m.w().stops.contains(&"decider"));
    }

    #[test]
    fn stop_on_exit_stops_only_managed_servers() {
        let mut w = installed_all();
        w.healthy.insert("decider", true);
        let m = Mock::new(w);
        let (sup, lines) = start(&m, false);
        drop(sup); // Ctrl-C / q path ends in drop or stop_all
        assert_eq!(m.w().stops, vec!["fm-serve"]);
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == "stopped fm serve (Apple Foundation Models)"));
    }

    #[test]
    fn stop_all_is_idempotent() {
        let m = Mock::new(installed_all());
        let (mut sup, _) = start(&m, false);
        sup.stop_all();
        sup.stop_all();
        drop(sup);
        assert_eq!(m.w().stops, vec!["fm-serve", "decider"]);
    }

    #[test]
    fn no_launch_uses_only_running_servers() {
        let mut w = installed_all();
        w.healthy.insert("decider", true);
        let m = Mock::new(w);
        let (mut sup, lines) = start(&m, true);
        assert!(m.w().launches.is_empty());
        let tags = sup.tags();
        assert_eq!(tags.get(decider().base), Some(&"external"));
        assert_eq!(tags.get(fm().base), None);
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("--no-launch")));
        m.advance(Duration::from_secs(120));
        sup.tick();
        assert!(
            m.w().launches.is_empty(),
            "tick never launches under --no-launch"
        );
    }

    #[test]
    fn not_installed_is_not_launched() {
        let m = Mock::new(World::default());
        let (sup, _) = start(&m, false);
        assert!(m.w().launches.is_empty());
        assert!(sup.tags().is_empty());
    }

    #[test]
    fn repeated_ticks_never_double_launch_a_healthy_server() {
        let m = Mock::new(installed_all());
        let (mut sup, _) = start(&m, false);
        for _ in 0..20 {
            m.advance(Duration::from_secs(2));
            sup.tick();
        }
        assert_eq!(m.w().launches, vec!["fm-serve", "decider"]);
    }

    #[test]
    fn a_crashed_managed_server_restarts_with_doubling_backoff() {
        let mut w = installed_all();
        w.healthy.insert("decider", true); // external: never touched
        let m = Mock::new(w);
        let (mut sup, lines) = start(&m, false);
        let fm_launches = |m: &Mock| m.w().launches.iter().filter(|i| **i == "fm-serve").count();

        m.crash("fm-serve");
        sup.tick(); // notices the exit, schedules +1s
        assert_eq!(fm_launches(&m), 1);
        m.advance(Duration::from_millis(999));
        sup.tick();
        assert_eq!(fm_launches(&m), 1, "not before the backoff");
        m.advance(Duration::from_millis(1));
        sup.tick();
        assert_eq!(fm_launches(&m), 2, "restarted after 1s");

        m.crash("fm-serve");
        sup.tick(); // second crash: +2s
        m.advance(Duration::from_secs(1));
        sup.tick();
        assert_eq!(fm_launches(&m), 2);
        m.advance(Duration::from_secs(1));
        sup.tick();
        assert_eq!(fm_launches(&m), 3, "restarted after 2s");

        let lines = lines.lock().unwrap();
        assert!(lines
            .iter()
            .any(|l| l == "fm serve (Apple Foundation Models) exited; restarting in 2s"));
        assert!(lines
            .iter()
            .any(|l| l == "restarted fm serve (Apple Foundation Models) (attempt 2)"));
        assert!(!m.w().launches.contains(&"decider"));
    }

    #[test]
    fn backoff_resets_after_a_stable_minute() {
        let m = Mock::new(installed_all());
        let (mut sup, _) = start(&m, false);
        m.crash("fm-serve");
        sup.tick();
        m.advance(Duration::from_secs(1));
        sup.tick(); // restarted, restarts = 1
        m.advance(STABLE_AFTER);
        sup.tick(); // healthy for a minute: reset
        m.crash("fm-serve");
        sup.tick();
        m.advance(Duration::from_secs(1));
        sup.tick();
        assert_eq!(
            m.w().launches.iter().filter(|i| **i == "fm-serve").count(),
            3,
            "back to the 1s first delay"
        );
    }

    #[test]
    fn restart_delay_doubles_and_caps() {
        assert_eq!(restart_delay(0), Duration::from_secs(1));
        assert_eq!(restart_delay(1), Duration::from_secs(2));
        assert_eq!(restart_delay(5), Duration::from_secs(32));
        assert_eq!(restart_delay(6), RESTART_MAX);
        assert_eq!(restart_delay(40), RESTART_MAX);
    }

    #[test]
    fn start_waits_for_a_slow_server_then_moves_on() {
        let mut w = installed_all();
        w.slow.push("decider");
        let m = Mock::new(w);
        let (mut sup, lines) = start(&m, false);
        // Waited the Decider's full timeout on the virtual clock, no longer.
        let waited = m.w().clock;
        assert!(waited >= decider().ready_timeout, "{waited:?}");
        assert!(waited < decider().ready_timeout + Duration::from_secs(1));
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("still starting")));
        // When it finally answers, tick asks for a rescan.
        m.w().healthy.insert("decider", true);
        assert!(sup.tick());
        assert!(!sup.tick(), "only once");
    }

    #[test]
    fn a_failed_launch_is_off_and_not_stopped() {
        let mut w = installed_all();
        w.fail_launch.push("fm-serve");
        let m = Mock::new(w);
        let (sup, lines) = start(&m, false);
        assert_eq!(sup.tags().get(fm().base), None);
        drop(sup);
        assert!(!m.w().stops.contains(&"fm-serve"));
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == "could not start fm serve (Apple Foundation Models): boom"));
    }

    #[test]
    fn child_pattern_matches_fm_serve_by_any_path() {
        assert_eq!(child_pattern("fm", &["serve"]), "(^|/)fm serve( |$)");
    }
}
