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
        let finder = if cfg!(windows) { "where" } else { "which" };
        let output = Command::new(finder).arg(command).output().ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
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
}
