//! `anyr relay doctor`: one place that says why nothing is served, with the
//! exact fix for each failure. Probes sit behind `Probe` so the checks are
//! testable off-Mac and offline.

use serde_json::{json, Value};

use super::view::Paint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Level {
    Ok,
    /// Optional or informational: does not fail the doctor run.
    Warn,
    Fail,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warn => "warn",
            Level::Fail => "fail",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Check {
    pub name: &'static str,
    pub level: Level,
    pub detail: String,
    /// Empty when there is nothing to fix.
    pub fix: String,
}

/// What the cloud said about the stored credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Auth {
    None,
    /// rk_ pairing token: no cheap way to verify it without connecting.
    Token,
    KeyOk,
    KeyRejected(u16),
    /// The check itself failed (offline, 5xx).
    Unverified(String),
}

pub(super) trait Probe {
    fn is_mac(&self) -> bool;
    fn macos_major(&self) -> Option<u32>;
    fn fm_serve(&self) -> bool;
    fn decider(&self) -> bool;
    /// Labels of other local servers that answered (Ollama, LM Studio, …).
    fn other_servers(&self) -> Vec<String>;
    fn network(&self) -> Result<(), String>;
    fn auth(&self) -> Auth;
    fn relay_pid(&self) -> Option<u32>;
}

fn check(
    name: &'static str,
    level: Level,
    detail: impl Into<String>,
    fix: impl Into<String>,
) -> Check {
    Check {
        name,
        level,
        detail: detail.into(),
        fix: fix.into(),
    }
}

pub(super) fn run_checks(p: &dyn Probe, bin: &str) -> Vec<Check> {
    let mut out = Vec::new();
    let mac = p.is_mac();
    let major = p.macos_major();

    let apple_ok = match (mac, major) {
        (false, _) => {
            out.push(check(
                "macOS",
                Level::Warn,
                "not a Mac: Apple Foundation Models are unavailable here",
                "serve Ollama or LM Studio, or pass --target <url>",
            ));
            false
        }
        (true, Some(v)) if v < 26 => {
            out.push(check(
                "macOS",
                Level::Warn,
                format!("macOS {v}: Apple Foundation Models need macOS 26 or later"),
                "update in System Settings > General > Software Update",
            ));
            false
        }
        (true, v) => {
            out.push(check(
                "macOS",
                Level::Ok,
                v.map(|v| format!("macOS {v}"))
                    .unwrap_or_else(|| "macOS".into()),
                "",
            ));
            true
        }
    };

    let fm = apple_ok && p.fm_serve();
    if apple_ok {
        out.push(if fm {
            check("fm serve", Level::Ok, "listening on 127.0.0.1:1976", "")
        } else {
            check(
                "fm serve",
                Level::Warn,
                "not listening on 127.0.0.1:1976",
                "turn on Apple Intelligence, then run `fm serve`",
            )
        });
    }

    let decider = p.decider();
    out.push(if decider {
        check("Decider app", Level::Ok, "answering on 127.0.0.1:8080", "")
    } else {
        check(
            "Decider app",
            Level::Warn,
            "not answering on 127.0.0.1:8080",
            "open the Decider for Mac app",
        )
    });

    let others = p.other_servers();
    if !others.is_empty() {
        out.push(check("Local servers", Level::Ok, others.join(", "), ""));
    }
    if !fm && !decider && others.is_empty() {
        out.push(check(
            "Models",
            Level::Fail,
            "nothing on this machine can be served",
            "run `fm serve`, open the Decider app, or start Ollama / LM Studio",
        ));
    }

    out.push(match p.network() {
        Ok(()) => check("Network", Level::Ok, "anyrouter.dev reachable", ""),
        Err(e) => check(
            "Network",
            Level::Fail,
            format!("cannot reach anyrouter.dev: {e}"),
            "check your connection, VPN or HTTPS_PROXY",
        ),
    });

    out.push(match p.auth() {
        Auth::None => check(
            "Auth",
            Level::Fail,
            "no AnyRouter key or relay token",
            format!("`{bin} login`"),
        ),
        Auth::Token => check("Auth", Level::Ok, "relay pairing token stored", ""),
        Auth::KeyOk => check("Auth", Level::Ok, "AnyRouter key accepted", ""),
        Auth::KeyRejected(code) => check(
            "Auth",
            Level::Fail,
            format!("AnyRouter key rejected ({code})"),
            format!("`{bin} login` for a fresh key"),
        ),
        Auth::Unverified(e) => check(
            "Auth",
            Level::Warn,
            format!("key stored but not verified: {e}"),
            format!("re-run `{bin} relay doctor` once online"),
        ),
    });

    out.push(match p.relay_pid() {
        Some(pid) => check("Relay", Level::Ok, format!("running (pid {pid})"), ""),
        None => check(
            "Relay",
            Level::Warn,
            "not running",
            format!("`{bin} relay start --detach`"),
        ),
    });
    out
}

pub(super) fn failed(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.level == Level::Fail)
}

pub(super) fn render(checks: &[Check], p: Paint) -> String {
    let w = checks.iter().map(|c| c.name.len()).max().unwrap_or(0);
    let mut out = String::new();
    for c in checks {
        let mark = match c.level {
            Level::Ok => p.ok("✓"),
            Level::Warn => p.warn("!"),
            Level::Fail => p.bad("✗"),
        };
        out += &format!("  {mark} {:<w$}  {}\n", c.name, c.detail);
        if !c.fix.is_empty() {
            out += &format!("    {:<w$}  {} {}\n", "", p.dim("fix:"), c.fix);
        }
    }
    out
}

pub(super) fn to_json(checks: &[Check]) -> Value {
    json!({
        "ok": !failed(checks),
        "checks": checks.iter().map(|c| json!({
            "name": c.name,
            "status": c.level.name(),
            "detail": c.detail,
            "fix": if c.fix.is_empty() { Value::Null } else { json!(c.fix) },
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        mac: bool,
        major: Option<u32>,
        fm: bool,
        decider: bool,
        others: Vec<String>,
        net: Result<(), String>,
        auth: Auth,
        pid: Option<u32>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Fake {
                mac: true,
                major: Some(26),
                fm: true,
                decider: true,
                others: vec![],
                net: Ok(()),
                auth: Auth::KeyOk,
                pid: Some(42),
            }
        }
    }

    impl Probe for Fake {
        fn is_mac(&self) -> bool {
            self.mac
        }
        fn macos_major(&self) -> Option<u32> {
            self.major
        }
        fn fm_serve(&self) -> bool {
            self.fm
        }
        fn decider(&self) -> bool {
            self.decider
        }
        fn other_servers(&self) -> Vec<String> {
            self.others.clone()
        }
        fn network(&self) -> Result<(), String> {
            self.net.clone()
        }
        fn auth(&self) -> Auth {
            self.auth.clone()
        }
        fn relay_pid(&self) -> Option<u32> {
            self.pid
        }
    }

    fn get<'a>(checks: &'a [Check], name: &str) -> &'a Check {
        checks.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn healthy_mac_passes_every_check() {
        let c = run_checks(&Fake::default(), "anyr");
        assert!(!failed(&c));
        assert!(c.iter().all(|c| c.level == Level::Ok), "{c:?}");
    }

    #[test]
    fn old_macos_skips_fm_serve_and_says_how_to_update() {
        let c = run_checks(
            &Fake {
                major: Some(15),
                ..Fake::default()
            },
            "anyr",
        );
        assert!(get(&c, "macOS").fix.contains("Software Update"));
        assert!(c.iter().all(|c| c.name != "fm serve"));
        // Decider still serves, so this is not a failure.
        assert!(!failed(&c));
    }

    #[test]
    fn nothing_servable_fails_with_a_fix_for_each_source() {
        let c = run_checks(
            &Fake {
                fm: false,
                decider: false,
                ..Fake::default()
            },
            "anyr",
        );
        assert!(failed(&c));
        assert!(get(&c, "fm serve").fix.contains("fm serve"));
        assert!(get(&c, "Decider app").fix.contains("Decider"));
        assert_eq!(get(&c, "Models").level, Level::Fail);
    }

    #[test]
    fn linux_with_ollama_is_healthy() {
        let c = run_checks(
            &Fake {
                mac: false,
                major: None,
                fm: false,
                decider: false,
                others: vec!["Ollama".into()],
                ..Fake::default()
            },
            "anyr",
        );
        assert!(!failed(&c), "{c:?}");
        assert_eq!(get(&c, "macOS").level, Level::Warn);
    }

    #[test]
    fn auth_and_network_failures_name_the_fix_command() {
        let c = run_checks(
            &Fake {
                auth: Auth::None,
                net: Err("dns".into()),
                pid: None,
                ..Fake::default()
            },
            "anyr",
        );
        assert!(failed(&c));
        assert_eq!(get(&c, "Auth").fix, "`anyr login`");
        assert!(get(&c, "Network").detail.contains("dns"));
        assert!(get(&c, "Relay").fix.contains("relay start --detach"));
        let c = run_checks(
            &Fake {
                auth: Auth::KeyRejected(401),
                ..Fake::default()
            },
            "anyr",
        );
        assert!(get(&c, "Auth").detail.contains("401"));
        // Offline auth check is a warning, not a hard failure.
        let c = run_checks(
            &Fake {
                auth: Auth::Unverified("timeout".into()),
                ..Fake::default()
            },
            "anyr",
        );
        assert_eq!(get(&c, "Auth").level, Level::Warn);
    }

    #[test]
    fn render_and_json_snapshot() {
        let c = vec![
            check("macOS", Level::Ok, "macOS 26", ""),
            check("Auth", Level::Fail, "no key", "`anyr login`"),
        ];
        assert_eq!(
            render(&c, Paint(false)),
            "  ✓ macOS  macOS 26\n  ✗ Auth   no key\n           fix: `anyr login`\n"
        );
        let j = to_json(&c);
        assert_eq!(j["ok"], false);
        assert_eq!(j["checks"][0]["fix"], Value::Null);
        assert_eq!(j["checks"][1]["status"], "fail");
    }
}
