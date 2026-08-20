use std::cell::RefCell;
use std::path::Path;

use crate::VERSION;

thread_local! {
    static INVOKED_BIN: RefCell<String> = RefCell::new(String::new());
}

const LAUNCH_HELP_BODY: &str = "\
Running this with no flags opens the launcher (review settings, then start).
Add --ok to skip the launcher and start with current settings.

Options:
  --ok, --yes           Skip the launcher and start with current settings
  --no-check            Skip the pre-launch reachability probe
  --model auto|<id>     Model for this session — \"auto\" picks from the preset
  --effort <level>      Reasoning effort: minimal | low | medium | high | xhigh | max
  --hub <slug>          Load a hub: sync ~/.anyrouter/hubs + claude --plugin-dir
  --profile <name>      Use a named profile
  --command-path <path> Explicit path to the agent executable
  --install             If the agent isn't installed, install it (skip the prompt)
  --config <path>       Override the config file path
  --dry-run             Print the child command and env (secrets redacted)
  --device              Force the device-code flow (headless / SSH)
  --device-code         Alias of --device
  --paste               Force the paste-a-key flow, skipping auto-detection

First run with no config auto-detects the best way to sign in — browser first,
falling back to the device-code flow automatically.
";

/// Map argv0 / npm shim name to the command users should type in help.
pub fn display_bin(argv0: &str) -> String {
    let name = Path::new(argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(argv0);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    match name {
        "ar" => "ar".into(),
        "anyrouter" => "anyrouter".into(),
        "anyr" => "anyr".into(),
        "cli" | "npx-anyr" | "npx-anyr.js" => "npx @anyr/cli".into(),
        other if other.starts_with("npx @anyr/cli") => "npx @anyr/cli".into(),
        other if other.starts_with("anyr-") => "anyr".into(),
        _ => "anyr".into(),
    }
}

pub fn set_invoked_bin(name: impl Into<String>) {
    let name = name.into();
    INVOKED_BIN.with(|slot| *slot.borrow_mut() = name);
}

pub fn invoked_bin() -> String {
    INVOKED_BIN.with(|slot| {
        let current = slot.borrow();
        if current.is_empty() {
            "anyr".into()
        } else {
            current.clone()
        }
    })
}

/// Resolve the help command from env (`ANYR_DISPLAY_BIN`) or argv0.
pub fn resolve_bin(argv0: Option<&str>, display_env: Option<&str>) -> String {
    if let Some(v) = display_env.map(str::trim).filter(|s| !s.is_empty()) {
        return display_bin(v);
    }
    display_bin(argv0.unwrap_or("anyr"))
}

pub fn root_help() -> String {
    let bin = invoked_bin();
    format!(
        "\
AnyRouter CLI v{version} — https://anyrouter.dev

Launch coding agents through the AnyRouter gateway. One key, every provider.

Install:
  curl -fsSL https://anyrouter.dev/setup.sh | bash

Quick start:
  {bin}                 Open the launcher on your last agent (no args)
  {bin} chat            Chat with any model in your terminal (streaming)
  {bin} claude          Open the Claude Code launcher (review, then start)
  {bin} claude --ok     Start Claude Code now with current settings
  {bin} codex           Open the Codex launcher
  {bin} opencode        Open the OpenCode launcher
  {bin} pi              Open the Pi launcher
  {bin} cursor          Print the config to paste into Cursor

Usage:
  {bin} <command> [options]
  {bin} <command> --help

Commands:
  menu      Open the interactive TUI (default when no command given)
  login     Sign in: open browser, paste key, save locally
  logout    Remove the stored keys for an account
  whoami    Show the active account + masked credentials (\"status\" works too)
  audit     Effective config, key storage, redacted env injection, launch history
  account   Manage multiple accounts (list | use | add | rename | remove)
  setup     Save a local profile (paste a key or log in)
  chat      Chat with any model in your terminal (streaming TUI)
  task      Plan with one model, implement with another (plan → implement)
  delegate  Hand a task to a coding agent headlessly (--to claude|codex|opencode|pi)
  relay     Relay: start a local LLM server relay or pair this device

Launch agents (spawn through AnyRouter):
  claude    Launch Claude Code routed through AnyRouter
  cc        Alias of \"claude\"
  codex     Launch Codex routed through AnyRouter
  grok      Launch Grok Build routed through AnyRouter
  opencode  Launch opencode routed through AnyRouter
  pi        Launch Pi routed through AnyRouter
  pool      Launch Poolside routed through AnyRouter
  poolside  Alias of \"pool\"

Editors (print config to paste — not launched):
  cursor    Print the AnyRouter base URL + key for Cursor
  cline     Print the AnyRouter base URL + key for Cline
  windsurf  Print the AnyRouter base URL + key for Windsurf

  models    List catalog model ids usable with --model
  usage     Credit balance, 24h spend + top models, lifetime used (cached on fail)
  logs      Recent requests: time, status, model, tokens, cost, latency
  transactions  Credit ledger: top-ups, spend, bonuses (newest first)
  keys      Manage API keys (list | create | use | revoke; bare = dashboard)
  config    Inspect, locate, or switch profiles (get | path | use)
  skills    Sync your Skills & Knowledge Hub locally (sync | pull | list | add | push)
  prompt    Pull prompts from the AnyRouter hub (get | url | list)
  byok        Add BYOK keys (byok add <provider> | byok locate <provider>)
  upgrade   Install the latest anyr from GitHub releases (alias: update)
  update    Alias of upgrade
  help      Show this help

Options:
  --ok, --yes              Skip the launcher and start with current settings
  --no-check               Skip the pre-launch reachability probe
  --model auto|<id>        One-session model: \"auto\" picks from the pinned preset
  --effort <level>         Reasoning effort: minimal | low | medium | high | xhigh | max
  --hub <slug>             Load a hub: sync ~/.anyrouter/hubs + claude --plugin-dir
  --preset <slug>          Preset to pin (e.g. coding-stack)
  --key sk-ar-v1-...       AnyRouter API key (setup / first run)
  --management-key ak_...  Management key, enables preset validation
  --profile <name>         Use a named profile (default: \"default\")
  --base-url <url>         Override the AnyRouter base URL
  --command-path <path>    Explicit path to the agent executable
  --config <path>          Override the config file path
  --plaintext              Store keys in config.yaml instead of the OS keychain
  --dry-run                Print the child command and env (secrets redacted)
  -v, --version            Print the CLI version

Docs: https://anyrouter.dev/docs/cli
",
        version = VERSION,
        bin = bin
    )
}

fn launch_help(bin: &str, id: &str, label: &str) -> String {
    format!(
        "{bin} {id} — launch {label} through AnyRouter\n\nUsage:\n  {bin} {id} [options] [-- <{id}-args>]\n\n{LAUNCH_HELP_BODY}"
    )
}

fn fill(bin: &str, template: &str) -> String {
    template.replace("{bin}", bin)
}

pub fn command_help(command: &str) -> Option<String> {
    let bin = invoked_bin();
    let canonical = match command {
        "cc" => "claude",
        "poolside" => "pool",
        "status" => "whoami",
        "update" => "upgrade",
        other => other,
    };
    Some(match canonical {
        "login" => fill(&bin, LOGIN),
        "setup" => fill(&bin, SETUP),
        "usage" => fill(&bin, USAGE),
        "whoami" => fill(&bin, WHOAMI),
        "account" => fill(&bin, ACCOUNT),
        "logs" => fill(&bin, LOGS),
        "models" => fill(&bin, MODELS),
        "config" => fill(&bin, CONFIG),
        "chat" => fill(&bin, CHAT),
        "skills" => fill(&bin, SKILLS),
        "relay" => fill(&bin, RELAY),
        "byok" => fill(&bin, BYOK),
        "task" => fill(&bin, TASK),
        "delegate" => fill(&bin, DELEGATE),
        "keys" => fill(&bin, KEYS),
        "audit" => fill(&bin, AUDIT),
        "logout" => fill(&bin, LOGOUT),
        "upgrade" => fill(&bin, UPGRADE),
        "transactions" => fill(&bin, TRANSACTIONS),
        "menu" => fill(
            &bin,
            "{bin} menu — open the interactive TUI (launch, switch model, switch key, credits)\n",
        ),
        "prompt" => fill(
            &bin,
            "{bin} prompt — pull prompts from the AnyRouter hub (get | url | list)\n",
        ),
        "claude" => launch_help(&bin, "claude", "Claude Code"),
        "codex" => launch_help(&bin, "codex", "Codex"),
        "grok" => launch_help(&bin, "grok", "Grok Build"),
        "opencode" => launch_help(&bin, "opencode", "opencode"),
        "pi" => launch_help(&bin, "pi", "Pi"),
        "pool" => launch_help(&bin, "pool", "Poolside"),
        "cursor" | "cline" | "windsurf" => format!(
            "{bin} {canonical} — print the AnyRouter base URL + key to paste into the editor\n"
        ),
        _ => return None,
    })
}

const LOGIN: &str = "\
{bin} login — sign in and save your AnyRouter API key

Usage:
  {bin} login [--key sk-ar-v1-...] [options]

Interactive (TTY): auto-detects the best way in — opens your browser (PKCE,
never prints the full key — only a prefix…suffix) when one looks reachable,
and falls back to the device-code flow automatically over SSH, in CI, or if
the browser handoff doesn't complete. Force a specific route with --device or
--paste.

Non-interactive: pass --key or set ANYROUTER_API_KEY.

Options:
  --key sk-ar-v1-...       AnyRouter API key (skips the prompt)
  --device, --device-code  Force the device-code flow (headless / SSH)
  --paste                  Force the paste-a-key flow, skipping auto-detection
  --yes                    Skip the post-login model/agent wizard
";

const SETUP: &str = "\
{bin} setup — save a local AnyRouter profile

Usage:
  {bin} setup [--key sk-ar-v1-...] [options]

Non-interactive: pass --key or set ANYROUTER_API_KEY.

Options:
  --key sk-ar-v1-...       AnyRouter API key (skips the prompt)
  --yes                    Skip prompts
";

const USAGE: &str = "\
{bin} usage — credits remaining, 24h spend, top models

Usage:
  {bin} usage [options]

Options:
  --json            Print as JSON
  --key sk-ar-v1-…  Use this key instead of the saved profile
  --profile <name>  Use a named profile
";

const WHOAMI: &str = "\
{bin} whoami — active account

Usage:
  {bin} whoami [--json]

Shows the active account + masked credentials. \"status\" is an alias.

Options:
  --json           Print as JSON
";

const ACCOUNT: &str = "\
{bin} account — manage multiple accounts

Usage:
  {bin} account list
  {bin} account use <name>
  {bin} account add [--yes]

Options:
  --yes            Skip confirmations
";

const LOGS: &str = "\
{bin} logs — recent requests

Usage:
  {bin} logs [options]

Lists recent requests: time, status, model, tokens, cost, latency.

Options:
  --limit <n>       Rows to fetch
  --status <s>      success | error | rejected
";

const MODELS: &str = "\
{bin} models — list catalog model ids

Usage:
  {bin} models [options]
  {bin} models use <id>
  {bin} models --pick

Lists every model id usable with --model. `use` / `--pick` persist default_model.

Options:
  --json            Print as JSON
  --pick            Interactive picker (TTY)
  --key sk-ar-v1-…  Optional inference key
";

const CONFIG: &str = "\
{bin} config — inspect and switch profiles

Usage:
  {bin} config get [--json]
  {bin} config path
  {bin} config use <profile>
";

const CHAT: &str = "\
{bin} chat — chat with any model in your terminal

Usage:
  {bin} chat [options]

Options:
  --model auto|<id>   Initial model
  --effort <level>    Reasoning effort
  --key sk-ar-v1-...  AnyRouter API key
";

const SKILLS: &str = "\
{bin} skills — sync your Skills & Knowledge Hub locally

Usage:
  {bin} skills sync
  {bin} skills pull <hub>
  {bin} skills list

Options:
  --hub <slug>     Hub slug
  --dry-run        Show what would change without writing
";

const RELAY: &str = "\
{bin} relay start [--target <url>] [--token <rk_...>] [--pool]
  {bin} relay pair [--name \"My Mac\"]
";

const BYOK: &str = "\
{bin} byok locate antigravity
{bin} byok add antigravity --yes
";

const TASK: &str = "\
{bin} task \"<x>\" — two-phase plan → implement

Options:
  --plan-model <id>
  --do-model <id>
  --yes
";

const DELEGATE: &str = "\
{bin} delegate --to claude|codex|opencode|pi \"<task>\"

Options:
  --to <agent>
  --yes
  --dry-run
";

const KEYS: &str = "\
{bin} keys — manage API keys

Usage:
  {bin} keys list [--json]
  {bin} keys create [name]
  {bin} keys use [hash]
  {bin} keys revoke <hash> [--yes]

Needs a management key (ak_…) from device/browser login, or --management-key.
";

const AUDIT: &str = "\
{bin} audit — see everything the CLI has configured and done

Options:
  --launches
  --tool <id>
  --json
";

const LOGOUT: &str = "\
{bin} logout — remove the stored keys for an account
";

const UPGRADE: &str = "\
{bin} upgrade — install the latest CLI from GitHub Releases

Usage:
  {bin} upgrade [--check] [--channel stable|beta] [--dry-run]

Channels:
  stable  (default)  latest non-prerelease
  beta               latest GitHub prerelease

Downloads:
  https://github.com/anyrouter-dev/cli/releases/download/<tag>/anyr-<os>-<arch>

--check reports current vs latest without installing.
--fixture <path> / ANYR_RELEASES_JSON skips the network (tests / dry-run).
ANYR_CHANNEL=stable|beta selects the channel when --channel is omitted.
";

const TRANSACTIONS: &str = "\
{bin} transactions — credit grants, top-ups, and spend

Options:
  --limit <n>
  --type <t>
  --json
  --key sk-ar-v1-…
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_bin_from_symlink_and_npx() {
        assert_eq!(display_bin("/home/duyet/.local/bin/ar"), "ar");
        assert_eq!(display_bin("anyrouter"), "anyrouter");
        assert_eq!(display_bin("anyr"), "anyr");
        assert_eq!(display_bin("anyr.exe"), "anyr");
        assert_eq!(display_bin("cli"), "npx @anyr/cli");
        assert_eq!(display_bin("npx-anyr.js"), "npx @anyr/cli");
        assert_eq!(display_bin("anyr-linux-x86_64"), "anyr");
        assert_eq!(display_bin("npx @anyr/cli"), "npx @anyr/cli");
    }

    #[test]
    fn root_help_uses_invoked_name() {
        set_invoked_bin("ar");
        let out = root_help();
        assert!(out.contains("ar claude"), "{out}");
        assert!(out.contains("ar <command>"), "{out}");
        assert!(!out.contains("npx @anyr/cli"), "{out}");
        assert!(out.contains("https://anyrouter.dev/setup.sh"), "{out}");

        set_invoked_bin("npx @anyr/cli");
        let npx = root_help();
        assert!(npx.contains("npx @anyr/cli claude"), "{npx}");
        assert!(npx.contains("npx @anyr/cli <command>"), "{npx}");
        set_invoked_bin("anyr");
    }

    #[test]
    fn command_help_uses_invoked_name() {
        set_invoked_bin("ar");
        let login = command_help("login").unwrap();
        assert!(login.contains("ar login"), "{login}");
        assert!(!login.contains("npx @anyr/cli"), "{login}");
        let claude = command_help("claude").unwrap();
        assert!(claude.contains("ar claude"), "{claude}");
        set_invoked_bin("anyr");
    }

    #[test]
    fn resolve_bin_prefers_env() {
        assert_eq!(resolve_bin(Some("/usr/bin/anyr"), Some("ar")), "ar");
        assert_eq!(resolve_bin(Some("/usr/bin/anyr"), None), "anyr");
    }
}
