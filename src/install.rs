//! Resolve a coding-agent binary, and optionally install it (`--install`).

#[cfg(feature = "native")]
use std::process::Command;

use crate::spawn::canonical_tool;
use crate::term;

#[derive(Debug, Clone, Copy)]
pub struct ToolHint {
    pub label: &'static str,
    pub install: &'static str,
    pub docs: &'static str,
    pub env: &'static str,
}

/// Agents the launcher/settings know about, in display order.
pub const KNOWN_AGENTS: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("grok", "Grok Build"),
    ("opencode", "OpenCode"),
    ("pi", "Pi"),
    ("pool", "Poolside"),
];

/// `ANYR_AGENTS` overrides PATH detection (tests / dump). Unset = probe PATH.
/// Empty / `-` / `none` = nothing installed. Comma list = those ids.
pub fn agents_override(env: &std::collections::BTreeMap<String, String>) -> Option<Vec<String>> {
    let raw = env.get("ANYR_AGENTS")?;
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "-" || trimmed.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    Some(
        trimmed
            .split(',')
            .map(|s| crate::spawn::canonical_tool(s.trim()).to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

/// Whether this agent can be launched: override list, `ANYROUTER_*_PATH`, or PATH.
pub fn agent_available(
    id: &str,
    command: &str,
    env: &std::collections::BTreeMap<String, String>,
) -> bool {
    if let Some(list) = agents_override(env) {
        let id = crate::spawn::canonical_tool(id);
        return list.iter().any(|s| s == id);
    }
    if let Some(hint) = tool_hint(id) {
        if env
            .get(hint.env)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
        {
            return true;
        }
    }
    resolve_executable(command).is_some()
}

pub fn available_agents(
    env: &std::collections::BTreeMap<String, String>,
    command_for: impl Fn(&str) -> String,
) -> Vec<(&'static str, &'static str)> {
    KNOWN_AGENTS
        .iter()
        .copied()
        .filter(|(id, _)| agent_available(id, &command_for(id), env))
        .collect()
}

pub fn missing_agents(
    env: &std::collections::BTreeMap<String, String>,
    command_for: impl Fn(&str) -> String,
) -> Vec<(&'static str, &'static str)> {
    KNOWN_AGENTS
        .iter()
        .copied()
        .filter(|(id, _)| !agent_available(id, &command_for(id), env))
        .collect()
}

pub fn tool_hint(tool: &str) -> Option<ToolHint> {
    Some(match canonical_tool(tool) {
        "claude" => ToolHint {
            label: "Claude Code",
            install: "npm install -g @anthropic-ai/claude-code",
            docs: "https://docs.claude.com/en/docs/claude-code",
            env: "ANYROUTER_CLAUDE_PATH",
        },
        "codex" => ToolHint {
            label: "Codex",
            install: "npm install -g @openai/codex",
            docs: "https://github.com/openai/codex",
            env: "ANYROUTER_CODEX_PATH",
        },
        "grok" => ToolHint {
            label: "Grok Build",
            install: "curl -fsSL https://x.ai/cli/install.sh | bash",
            docs: "https://docs.x.ai/build/overview",
            env: "ANYROUTER_GROK_PATH",
        },
        "pool" => ToolHint {
            label: "Poolside",
            install: "curl -fsSL https://downloads.poolside.ai/pool/install.sh | sh",
            docs: "https://docs.poolside.ai/cli/install",
            env: "ANYROUTER_POOL_PATH",
        },
        "opencode" => ToolHint {
            label: "OpenCode",
            install: "npm install -g opencode-ai",
            docs: "https://opencode.ai/docs",
            env: "ANYROUTER_OPENCODE_PATH",
        },
        "pi" => ToolHint {
            label: "Pi",
            install: "npm install -g @mariozechner/pi-coding-agent",
            docs: "https://github.com/mariozechner/pi-mono",
            env: "ANYROUTER_PI_PATH",
        },
        _ => return None,
    })
}

pub fn missing_tool_hint(command: &str, hint: ToolHint) -> String {
    format!(
        "Could not find the \"{command}\" binary for {} on your PATH.\n\n\
Install it:\n  {}\n  Docs: {}\n\n\
Already installed somewhere else? Point AnyRouter at it:\n\
  • flag: --command-path /path/to/{command}\n\
  • env:  {}=/path/to/{command}\n",
        hint.label, hint.install, hint.docs, hint.env
    )
}

pub fn resolve_executable(command: &str) -> Option<String> {
    if command.contains('/') || command.contains('\\') || command.starts_with('.') {
        return Some(command.to_string());
    }
    #[cfg(not(feature = "native"))]
    {
        let _ = command;
        return None;
    }
    #[cfg(feature = "native")]
    {
        thread_local! {
            static HITS: std::cell::RefCell<std::collections::HashMap<String, Option<String>>> =
                std::cell::RefCell::new(std::collections::HashMap::new());
        }
        HITS.with(|hits| {
            if let Some(cached) = hits.borrow().get(command) {
                return cached.clone();
            }
            let found = find_on_path(command);
            // Cache hits only. A miss must be retried after install
            // (PATH changes in-process; a cached None would always fail).
            if found.is_some() {
                hits.borrow_mut().insert(command.to_string(), found.clone());
            }
            found
        })
    }
}

/// Walk `PATH` in-process. Spawning `which`/`where` six times delayed the launcher.
#[cfg(feature = "native")]
fn find_on_path(command: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var_os("PATHEXT")
            .map(|v| {
                v.to_string_lossy()
                    .split(';')
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_else(|| {
                [".EXE", ".CMD", ".BAT", ".COM"]
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            })
    } else {
        Vec::new()
    };
    for dir in std::env::split_paths(&path) {
        let direct = dir.join(command);
        if is_runnable(&direct) {
            return Some(direct.to_string_lossy().into_owned());
        }
        for ext in &exts {
            if command.rsplit('.').next().is_some_and(|e| {
                !e.is_empty() && e.eq_ignore_ascii_case(ext.trim_start_matches('.'))
            }) {
                continue;
            }
            let candidate = dir.join(format!("{command}{ext}"));
            if is_runnable(&candidate) {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

#[cfg(feature = "native")]
fn is_runnable(path: &std::path::Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(not(feature = "native"))]
pub fn run_installer(_install_command: &str) -> bool {
    false
}

#[cfg(feature = "native")]
pub fn run_installer(install_command: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(install_command)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Locate `command` on PATH. If missing: `--install` (or a TTY y/N) runs the
/// documented installer, then re-resolves.
pub fn ensure_tool_installed(
    tool_name: &str,
    command: &str,
    auto_install: bool,
) -> Result<String, String> {
    if let Some(path) = resolve_executable(command) {
        return Ok(path);
    }
    let Some(hint) = tool_hint(tool_name) else {
        return Err(format!(
            "Missing child executable \"{command}\". Install it, or pass --command-path <path>."
        ));
    };
    let want = auto_install
        || (term::is_interactive()
            && term::confirm(&format!(
                "{} (\"{command}\") isn't on your PATH. Install it now?",
                hint.label
            )));
    if !want {
        return Err(missing_tool_hint(command, hint));
    }
    eprintln!(
        "{}",
        term::dim(&format!("Installing {}…\n  {}", hint.label, hint.install))
    );
    if !run_installer(hint.install) {
        return Err(format!(
            "Installation failed. Try installing it manually:\n  {}",
            hint.install
        ));
    }
    if let Some(path) = resolve_executable(command) {
        return Ok(path);
    }
    Err(format!(
        "{} was installed, but \"{command}\" still isn't on this shell's PATH.\n\n\
Open a new terminal (so PATH reloads), then re-run — or point AnyRouter at it directly:\n\
  • flag: --command-path /path/to/{command}\n\
  • env:  {}=/path/to/{command}\n",
        hint.label, hint.env
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_hint_uses_official_npm() {
        let hint = tool_hint("claude").unwrap();
        assert!(hint.install.contains("@anthropic-ai/claude-code"));
        assert!(missing_tool_hint("claude", hint).contains("Claude Code"));
    }

    #[test]
    fn cc_alias_resolves_to_claude_hint() {
        assert_eq!(tool_hint("cc").unwrap().label, "Claude Code");
    }

    #[test]
    fn relative_path_is_used_as_is() {
        assert_eq!(
            resolve_executable("./bin/claude").as_deref(),
            Some("./bin/claude")
        );
    }

    #[test]
    fn resolve_executable_finds_a_real_binary() {
        #[cfg(unix)]
        {
            let found = resolve_executable("true");
            assert!(
                found.as_deref().is_some_and(|p| p.ends_with("true")),
                "expected true on PATH, got {found:?}"
            );
        }
    }

    #[test]
    fn agents_override_empty_means_none() {
        let mut env = std::collections::BTreeMap::new();
        assert!(agents_override(&env).is_none());
        env.insert("ANYR_AGENTS".into(), "".into());
        assert_eq!(agents_override(&env), Some(vec![]));
        env.insert("ANYR_AGENTS".into(), "none".into());
        assert_eq!(agents_override(&env), Some(vec![]));
        env.insert("ANYR_AGENTS".into(), "claude, grok".into());
        assert_eq!(
            agents_override(&env),
            Some(vec!["claude".into(), "grok".into()])
        );
    }

    #[test]
    fn agent_available_honors_override() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("ANYR_AGENTS".into(), "codex".into());
        assert!(!agent_available("claude", "claude", &env));
        assert!(agent_available("codex", "codex", &env));
    }

    #[test]
    fn misses_are_not_cached_so_install_can_retry() {
        assert!(resolve_executable("anyr-definitely-not-on-path-xyz").is_none());
        #[cfg(unix)]
        {
            assert!(
                resolve_executable("true").is_some(),
                "a miss must not poison later lookups"
            );
        }
    }
}
