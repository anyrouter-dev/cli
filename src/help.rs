use crate::VERSION;

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

pub fn root_help() -> String {
    format!(
        "\
AnyRouter CLI v{version} — https://anyrouter.dev

Launch coding agents through the AnyRouter gateway. One key, every provider.

Install:
  curl -fsSL https://raw.githubusercontent.com/anyrouter-dev/cli/main/setup.sh | bash

Quick start:
  npx @anyr/cli                 Open the launcher on your last agent (no args)
  npx @anyr/cli chat            Chat with any model in your terminal (streaming)
  npx @anyr/cli claude          Open the Claude Code launcher (review, then start)
  npx @anyr/cli claude --ok     Start Claude Code now with current settings
  npx @anyr/cli codex           Open the Codex launcher
  npx @anyr/cli opencode        Open the OpenCode launcher
  npx @anyr/cli pi              Open the Pi launcher
  npx @anyr/cli cursor          Print the config to paste into Cursor

Usage:
  npx @anyr/cli <command> [options]
  npx @anyr/cli <command> --help

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
        version = VERSION
    )
}

fn launch_help(id: &str, label: &str) -> String {
    format!(
        "npx @anyr/cli {id} — launch {label} through AnyRouter\n\nUsage:\n  npx @anyr/cli {id} [options] [-- <{id}-args>]\n\n{LAUNCH_HELP_BODY}"
    )
}

pub fn command_help(command: &str) -> Option<String> {
    let canonical = match command {
        "cc" => "claude",
        "poolside" => "pool",
        "status" => "whoami",
        "update" => "upgrade",
        other => other,
    };
    Some(match canonical {
        "login" => LOGIN.into(),
        "setup" => SETUP.into(),
        "usage" => USAGE.into(),
        "whoami" => WHOAMI.into(),
        "account" => ACCOUNT.into(),
        "logs" => LOGS.into(),
        "models" => MODELS.into(),
        "config" => CONFIG.into(),
        "chat" => CHAT.into(),
        "skills" => SKILLS.into(),
        "relay" => RELAY.into(),
        "byok" => BYOK.into(),
        "task" => TASK.into(),
        "delegate" => DELEGATE.into(),
        "keys" => KEYS.into(),
        "audit" => AUDIT.into(),
        "logout" => LOGOUT.into(),
        "upgrade" => UPGRADE.into(),
        "transactions" => TRANSACTIONS.into(),
        "menu" => "anyrouter menu — open the interactive TUI (launch, switch model, switch key, credits)\n".into(),
        "prompt" => "anyrouter prompt — pull prompts from the AnyRouter hub (get | url | list)\n".into(),
        "claude" => launch_help("claude", "Claude Code"),
        "codex" => launch_help("codex", "Codex"),
        "grok" => launch_help("grok", "Grok Build"),
        "opencode" => launch_help("opencode", "opencode"),
        "pi" => launch_help("pi", "Pi"),
        "pool" => launch_help("pool", "Poolside"),
        "cursor" | "cline" | "windsurf" => format!(
            "npx @anyr/cli {canonical} — print the AnyRouter base URL + key to paste into the editor\n"
        ),
        _ => return None,
    })
}

const LOGIN: &str = "\
anyrouter login — sign in and save your AnyRouter API key

Usage:
  npx @anyr/cli login [--key sk-ar-v1-...] [options]

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
anyrouter setup — save a local AnyRouter profile

Usage:
  npx @anyr/cli setup [--key sk-ar-v1-...] [options]

Non-interactive: pass --key or set ANYROUTER_API_KEY.

Options:
  --key sk-ar-v1-...       AnyRouter API key (skips the prompt)
  --yes                    Skip prompts
";

const USAGE: &str = "\
anyrouter usage — credits remaining, 24h spend, top models

Usage:
  npx @anyr/cli usage [options]

Options:
  --json            Print as JSON
  --key sk-ar-v1-…  Use this key instead of the saved profile
  --profile <name>  Use a named profile
";

const WHOAMI: &str = "\
anyrouter whoami — active account

Usage:
  npx @anyr/cli whoami [--json]

Shows the active account + masked credentials. \"status\" is an alias.

Options:
  --json           Print as JSON
";

const ACCOUNT: &str = "\
anyrouter account — manage multiple accounts

Usage:
  npx @anyr/cli account list
  npx @anyr/cli account use <name>
  npx @anyr/cli account add [--yes]

Options:
  --yes            Skip confirmations
";

const LOGS: &str = "\
anyrouter logs — recent requests

Usage:
  npx @anyr/cli logs [options]

Lists recent requests: time, status, model, tokens, cost, latency.

Options:
  --limit <n>       Rows to fetch
  --status <s>      success | error | rejected
";

const MODELS: &str = "\
anyrouter models — list catalog model ids

Usage:
  npx @anyr/cli models [options]
  npx @anyr/cli models use <id>
  npx @anyr/cli models --pick

Lists every model id usable with --model. `use` / `--pick` persist default_model.

Options:
  --json            Print as JSON
  --pick            Interactive picker (TTY)
  --key sk-ar-v1-…  Optional inference key
";

const CONFIG: &str = "\
anyrouter config — inspect and switch profiles

Usage:
  npx @anyr/cli config get [--json]
  npx @anyr/cli config path
  npx @anyr/cli config use <profile>
";

const CHAT: &str = "\
anyrouter chat — chat with any model in your terminal

Usage:
  npx @anyr/cli chat [options]

Options:
  --model auto|<id>   Initial model
  --effort <level>    Reasoning effort
  --key sk-ar-v1-...  AnyRouter API key
";

const SKILLS: &str = "\
anyrouter skills — sync your Skills & Knowledge Hub locally

Usage:
  npx @anyr/cli skills sync
  npx @anyr/cli skills pull <hub>
  npx @anyr/cli skills list

Options:
  --hub <slug>     Hub slug
  --dry-run        Show what would change without writing
";

const RELAY: &str = "\
anyr relay start [--target <url>] [--token <rk_...>] [--pool]
  anyr relay pair [--name \"My Mac\"]
";

const BYOK: &str = "\
anyr byok locate antigravity
anyr byok add antigravity --yes
";

const TASK: &str = "\
anyr task \"<x>\" — two-phase plan → implement

Options:
  --plan-model <id>
  --do-model <id>
  --yes
";

const DELEGATE: &str = "\
anyr delegate --to claude|codex|opencode|pi \"<task>\"

Options:
  --to <agent>
  --yes
  --dry-run
";

const KEYS: &str = "\
anyr keys — manage API keys

Usage:
  anyr keys list [--json]
  anyr keys create [name]
  anyr keys use [hash]
  anyr keys revoke <hash> [--yes]

Needs a management key (ak_…) from device/browser login, or --management-key.
";

const AUDIT: &str = "\
anyrouter audit — see everything the CLI has configured and done

Options:
  --launches
  --tool <id>
  --json
";

const LOGOUT: &str = "\
anyrouter logout — remove the stored keys for an account
";

const UPGRADE: &str = "\
anyr upgrade — install the latest CLI from GitHub Releases

Usage:
  anyr upgrade [--check] [--channel stable|beta] [--dry-run]

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
anyrouter transactions — credit grants, top-ups, and spend

Options:
  --limit <n>
  --type <t>
  --json
  --key sk-ar-v1-…
";
