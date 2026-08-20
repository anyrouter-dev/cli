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
  --model auto|<id>     Session model. \"auto\" is anyrouter/auto (smart pick)
  --haiku <id>          Claude /model haiku and subagents
  --sonnet <id>         Claude /model sonnet
  --opus <id>           Claude /model opus
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
    let header = crate::term::brand_header(&[
        &format!(
            "{}  {}",
            crate::term::bold(&format!("AnyRouter CLI v{VERSION}")),
            crate::term::link("https://anyrouter.dev")
        ),
        &crate::term::dim("One key. Every coding agent."),
        "",
    ]);
    format!(
        "\
{header}
USAGE
  {bin} <command> [flags]
  {bin} <command> --help

CORE COMMANDS
  auth:       Authenticate with AnyRouter
  config:     Interactive settings (key, credits, model)
  keys:       Manage API keys
  models:     List catalog and set the default
  usage:      Credits remaining

LAUNCH
  claude    Claude Code
  cc        Alias of claude
  codex     Codex
  grok      Grok Build
  opencode  OpenCode
  pi        Pi
  pool      Poolside
  poolside  Alias of pool

  {bin}                 Sign in if needed, then open the launcher
  {bin} auth login      Sign in
  {bin} claude          Launch Claude Code
",
        header = header,
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
        "setup" => "login",
        other => other,
    };
    Some(match canonical {
        "auth" => fill(&bin, AUTH),
        "login" => fill(&bin, LOGIN),
        "token" => fill(&bin, TOKEN),
        "switch" => fill(&bin, SWITCH),
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

const AUTH: &str = "\
Authenticate with AnyRouter.

USAGE
  {bin} auth <command> [flags]

AVAILABLE COMMANDS
  login:       Log in to an AnyRouter account
  logout:      Log out of an AnyRouter account
  status:      View authentication status
  switch:      Switch the active account
  token:       Print the API key

FLAGS
  -h, --help   Show help for command
";

const LOGIN: &str = "\
Log in to an AnyRouter account.

USAGE
  {bin} auth login [flags]

Interactive (TTY): opens a login URL with the code already in it (browser
when one is reachable). Falls back to printing that URL over SSH / CI.
Force a route with --device or --paste.

Non-interactive: pass --key or set ANYROUTER_API_KEY.

FLAGS
  --key sk-ar-v1-...       AnyRouter API key (skips the prompt)
  --device, --device-code  Force the device-code flow (headless / SSH)
  --paste                  Force the paste-a-key flow
  --yes                    Skip the post-login model/agent wizard

Also available as `{bin} login`.
";

const TOKEN: &str = "\
Print the API key stored for the active account.

USAGE
  {bin} auth token [flags]

Prints the full secret to stdout (for scripts). Use --masked in a log.

FLAGS
  --masked     Print a redacted key
  --json       Print as JSON
  --profile    Use a named account
";

const SWITCH: &str = "\
Switch the active AnyRouter account.

USAGE
  {bin} auth switch [<account>]

With no account on a TTY, pick from stored accounts.

Also available as `{bin} account use <account>`.
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
View authentication status.

USAGE
  {bin} auth status [flags]

Shows the active account and masked credentials.

FLAGS
  --json       Print as JSON
  --profile    Use a named account

Also available as `{bin} whoami`.
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
  {bin} models use --haiku|--sonnet|--opus <id>
  {bin} models --pick

Lists every model id usable with --model. `use` / `--pick` persist the session
default, or Claude Code's opus / sonnet / haiku aliases.

Options:
  --json            Print as JSON
  --pick            Interactive picker (TTY) — default / haiku / sonnet / opus
  --haiku <id>      Persist Claude haiku alias
  --sonnet <id>     Persist Claude sonnet alias
  --opus <id>       Persist Claude opus alias
  --key sk-ar-v1-…  Optional inference key
";

const CONFIG: &str = "\
Interactive config: pick key, account, model, and see credits.

USAGE
  {bin} config                 Open the TUI (TTY)
  {bin} config get [--json]    Print current status
  {bin} config path            Print the config file path
  {bin} config use <account>   Switch the active account

On a TTY, `{bin} config` loops until you pick Done: switch key, account,
model (default / haiku / sonnet / opus), view credits, sign in, or log out.
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
Log out of an AnyRouter account.

USAGE
  {bin} auth logout [flags]

Removes stored keys for the active account (or --profile).

Also available as `{bin} logout`.
";

const UPGRADE: &str = "\
{bin} upgrade — install the latest CLI from GitHub Releases

Usage:
  {bin} upgrade [--check] [--channel stable|beta] [--dry-run]

Auto-update is on by default. On startup a background process checks
GitHub Releases, and while a coding agent is running it rechecks every
few hours, then installs in place. The next `{bin}` uses the new build.

  auto_update: false     in ~/.anyrouter/config.yaml to turn it off
  ANYR_AUTO_UPDATE=0     same, for this process
  {bin} config           toggle Auto-update in the TUI

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
        assert!(out.contains("ar auth login"), "{out}");
        assert!(out.contains("Sign in if needed"), "{out}");
        for heading in ["CORE COMMANDS", "LAUNCH"] {
            assert!(out.contains(heading), "missing {heading} in:\n{out}");
        }
        assert!(!out.contains("npx @anyr/cli"), "{out}");
        assert!(!out.contains("setup.sh"), "{out}");
        assert!(!out.contains("Install:"), "{out}");
        assert!(!out.contains("anyrouter.dev/docs/cli"), "{out}");
        assert!(
            out.contains("▀█████████▄"),
            "help should include the official AR half-block mark, got:\n{out}"
        );

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
        assert!(login.contains("ar auth login"), "{login}");
        assert!(!login.contains("npx @anyr/cli"), "{login}");
        let auth = command_help("auth").unwrap();
        assert!(auth.contains("ar auth <command>"), "{auth}");
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
