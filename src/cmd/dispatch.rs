use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::{resolve_config_path, write_config};
use crate::http::fetch_credits;
use crate::install::available_agents;
use crate::key::load_config_if_present;
use crate::parse::{get_string_flag, FlagValue, ParsedArgs};
use crate::spawn::{canonical_tool, resolve_tool};

#[cfg(not(feature = "native"))]
use crate::term;

#[cfg(feature = "native")]
pub(crate) fn tui_wants_dump(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> bool {
    crate::tui::wants_dump(parsed, env)
}

#[cfg(not(feature = "native"))]
pub(crate) fn tui_wants_dump(_parsed: &ParsedArgs, _env: &BTreeMap<String, String>) -> bool {
    false
}

#[cfg(not(feature = "native"))]
pub(crate) fn tui_menu_select(
    title: &str,
    _header: Vec<String>,
    items: Vec<String>,
) -> Result<Option<usize>, String> {
    match term::pick(title, &items, Some(0)) {
        Ok(i) => Ok(Some(i)),
        Err(_) => Ok(None),
    }
}

/// One launcher row as the inline fallback sees it. Native builds carry a
/// real `tui::PaletteEntry`; wasm / no-native builds use this plain twin so
/// the palette builder stays shared.
#[cfg(not(feature = "native"))]
#[derive(Clone)]
pub struct InlineEntry {
    pub label: String,
    pub detail: String,
    pub group: String,
    pub action: String,
}

#[cfg(not(feature = "native"))]
impl InlineEntry {
    pub(crate) fn new(
        label: impl Into<String>,
        detail: impl Into<String>,
        _group: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            group: String::new(),
            action: action.into(),
        }
    }
}

#[cfg(feature = "native")]
pub(crate) fn tui_palette_select(
    header: Vec<String>,
    entries: Vec<crate::tui::PaletteEntry>,
    on_idle: impl FnMut(&mut crate::tui::PaletteState),
) -> Result<Option<usize>, String> {
    crate::tui::run_palette_select_idle(header, entries, on_idle)
}

/// No fullscreen TUI (non-native build): the palette degrades to the inline
/// numbered-list picker — same entries, plain prompts.
#[cfg(feature = "native")]
pub(crate) fn pick_palette_action(
    header: Vec<String>,
    entries: Vec<crate::tui::PaletteEntry>,
    cache: &Arc<Mutex<CreditsCache>>,
) -> Result<Option<usize>, String> {
    let tick_cache = cache.clone();
    let mut seen = 0u64;
    tui_palette_select(header, entries, move |state| {
        let Ok(credits) = tick_cache.lock() else {
            return;
        };
        if credits.gen <= seen {
            return;
        }
        seen = credits.gen;
        patch_palette_header(state, &credits);
    })
}

#[cfg(not(feature = "native"))]
pub(crate) fn pick_palette_action(
    header: Vec<String>,
    entries: Vec<InlineEntry>,
    _cache: &Arc<Mutex<CreditsCache>>,
) -> Result<Option<usize>, String> {
    tui_palette_select(header, entries)
}

#[cfg(not(feature = "native"))]
pub(crate) fn tui_palette_select(
    _header: Vec<String>,
    entries: Vec<InlineEntry>,
) -> Result<Option<usize>, String> {
    let items: Vec<String> = entries
        .iter()
        .map(|e| {
            if e.detail.is_empty() {
                e.label.clone()
            } else {
                format!("{}  —  {}", e.label, e.detail)
            }
        })
        .collect();
    match term::pick("anyr", &items, Some(0)) {
        Ok(i) => Ok(Some(i)),
        Err(_) => Ok(None),
    }
}

/// Fullscreen palette only when the terminal can take it; dumb TTYs and
/// odd hosts get the inline list instead (Direction D fallback).
#[cfg(feature = "native")]
pub(crate) fn launcher_uses_palette() -> bool {
    crate::tui::can_use_fullscreen()
}

#[cfg(not(feature = "native"))]
pub(crate) fn launcher_uses_palette() -> bool {
    false
}

#[cfg(feature = "native")]
pub(crate) fn tui_dump_palette(
    entries: Vec<crate::tui::PaletteEntry>,
    header: Vec<String>,
    env: &BTreeMap<String, String>,
) -> String {
    crate::tui::dump_palette(
        &crate::tui::PaletteState::new(header, entries),
        crate::tui::dump_cols(env),
    )
}

#[cfg(not(feature = "native"))]
pub(crate) fn tui_dump_palette(
    entries: Vec<InlineEntry>,
    _header: Vec<String>,
    _env: &BTreeMap<String, String>,
) -> String {
    let mut lines = vec!["▲ anyr".to_string()];
    lines.extend(entries.iter().map(|e| format!("  {}", e.label)));
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(feature = "native")]
pub(crate) fn tui_settings_select(
    state: crate::tui::SettingsState,
) -> Result<Option<crate::tui::SettingsOutcome>, String> {
    if !crate::tui::is_interactive() {
        return Ok(None);
    }
    match crate::tui::run_settings_live(state)? {
        crate::tui::SettingsOutcome::Close | crate::tui::SettingsOutcome::Stay => Ok(None),
        outcome => Ok(Some(outcome)),
    }
}

#[cfg(feature = "native")]
pub(crate) fn tui_dump_settings(
    state: crate::tui::SettingsState,
    env: &BTreeMap<String, String>,
) -> String {
    crate::tui::dump_settings(&state, crate::tui::dump_cols(env))
}

pub(crate) const LAUNCH_FLAGS: &[&str] = &[
    "model",
    "effort",
    "haiku",
    "sonnet",
    "opus",
    "fable",
    "profile",
    "preset",
    "key",
    "base-url",
    "command-path",
    "claude-path",
    "config",
    "timeout",
    "plaintext",
    "dry-run",
    "yes",
    "ok",
    "no-check",
    "device",
    "device-code",
    "paste",
    "hub",
    "install",
    "yolo",
];

/// Command availability for dispatch / help honesty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmdKind {
    Implemented,
    Stub,
    HelpOnly,
}

pub(crate) fn cmd_kind(command: &str) -> Option<CmdKind> {
    let c = canonical_command(command);
    Some(match c {
        "setup" | "login" | "auth" | "menu" | "models" | "config" | "keys" | "whoami"
        | "status" | "logout" | "account" | "usage" | "claude" | "codex" | "grok" | "opencode"
        | "pool" | "pi" | "upgrade" | "onboard" | "impl" | "plan" | "fix" | "deploy" | "cp"
        | "relay" => CmdKind::Implemented,
        "cursor" | "cline" | "windsurf" => CmdKind::HelpOnly,
        "chat" | "task" | "delegate" | "audit" | "logs" | "transactions" | "skills" | "prompt"
        | "byok" => CmdKind::Stub,
        _ => return None,
    })
}

pub(crate) fn known_command(command: &str) -> bool {
    cmd_kind(command).is_some()
}

pub(crate) fn canonical_command(command: &str) -> &str {
    match command {
        "update" => "upgrade",
        "implement" => "impl",
        other => canonical_tool(other),
    }
}

pub(crate) fn allowed_flags(command: &str) -> Option<&'static [&'static str]> {
    let canonical = canonical_command(command);
    Some(match canonical {
        "login" | "setup" => &[
            "key",
            "preset",
            "profile",
            "base-url",
            "timeout",
            "config",
            "plaintext",
            "device",
            "device-code",
            "paste",
            "yes",
        ],
        "models" => &[
            "profile", "config", "json", "key", "base-url", "pick", "haiku", "sonnet", "opus",
            "fable", "agent", "dump-tui",
        ],
        "usage" => &["profile", "base-url", "config", "json", "key", "no-detail"],
        "whoami" => &["profile", "config", "json"],
        "config" => &["config", "json", "key", "base-url", "profile", "dump-tui"],
        "account" => &[
            "yes",
            "config",
            "profile",
            "key",
            "preset",
            "base-url",
            "timeout",
            "plaintext",
            "device",
            "device-code",
            "paste",
            "agent",
        ],
        "logs" => &[
            "profile", "base-url", "config", "json", "key", "limit", "model", "status",
        ],
        "transactions" => &[
            "profile", "base-url", "config", "json", "key", "limit", "type",
        ],
        "chat" => &[
            "model",
            "effort",
            "no-reasoning",
            "system",
            "preset",
            "key",
            "profile",
            "base-url",
            "config",
            "yes",
            "otui",
        ],
        "skills" => &["profile", "config", "hub", "json", "dry-run", "source"],
        "relay" => &[
            "target",
            "token",
            "url",
            "verbose",
            "name",
            "pool",
            "max-concurrency",
        ],
        "task" => &[
            "plan-model",
            "do-model",
            "profile",
            "base-url",
            "config",
            "key",
            "yes",
            "json",
        ],
        "delegate" => &[
            "to", "from", "model", "profile", "base-url", "config", "key", "yes", "dry-run",
        ],
        "keys" => &["profile", "base-url", "config", "json", "yes", "agent"],
        "audit" => &["profile", "config", "json", "launches", "tool", "limit"],
        "logout" => &["profile", "config"],
        "auth" => &[
            "key",
            "preset",
            "profile",
            "base-url",
            "timeout",
            "config",
            "plaintext",
            "device",
            "device-code",
            "paste",
            "yes",
            "json",
            "masked",
        ],
        "prompt" => &["base-url", "json"],
        "menu" => &["dump-tui", "config", "profile", "key", "base-url"],
        "onboard" | "impl" | "plan" | "fix" | "deploy" | "cp" => &["json", "copy"],
        "upgrade" => &[
            "check", "channel", "fixture", "dry-run", "yes", "auto", "force", "beta", "stable",
        ],
        "claude" | "codex" | "grok" | "opencode" | "pool" | "pi" => LAUNCH_FLAGS,
        "cursor" | "cline" | "windsurf" => &["profile", "key", "base-url", "config", "yes"],
        "byok" => return None,
        _ => return None,
    })
}

pub(crate) fn assert_known_flags(
    command: &str,
    flags: &HashMap<String, FlagValue>,
    allowed: &[&str],
) -> Result<(), String> {
    for name in flags.keys() {
        if name == "help" {
            continue;
        }
        if !allowed.contains(&name.as_str()) {
            return Err(format!("Unknown flag --{name} for \"{command}\"."));
        }
    }
    Ok(())
}

pub(crate) fn wants_help(parsed: &ParsedArgs) -> bool {
    parsed.flag_true("help")
        || parsed
            .passthrough
            .iter()
            .any(|a| a == "-h" || a == "--help")
}

pub(crate) fn help_topic(parsed: &ParsedArgs) -> String {
    match parsed.command.as_str() {
        "auth" => parsed
            .passthrough
            .iter()
            .find(|s| *s != "-h" && *s != "--help")
            .cloned()
            .unwrap_or_else(|| "auth".into()),
        other => other.to_string(),
    }
}

pub(crate) fn shift_passthrough(parsed: &ParsedArgs) -> ParsedArgs {
    let mut next = parsed.clone();
    if !next.passthrough.is_empty() {
        next.passthrough.remove(0);
    }
    next
}

pub(crate) fn hint(template: &str) -> String {
    template.replace("{bin}", &crate::help::invoked_bin())
}

pub(crate) fn stub(command: &str) -> Result<i32, String> {
    Err(format!(
        "\"{command}\" is not yet implemented in the native CLI. Run \"{} --help\" for available commands, or \"{} onboard\" for agent paste prompts.",
        crate::help::invoked_bin(),
        crate::help::invoked_bin(),
    ))
}

pub(crate) fn config_path(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> PathBuf {
    resolve_config_path(get_string_flag(&parsed.flags, "config").as_deref(), env)
}

/// Account-info cache for the launcher / settings loops: one fetch per TTL
/// instead of one per frame render. `None` = never fetched / failed.
pub(crate) struct CreditsCache {
    value: Option<Result<String, ()>>,
    me: Option<Result<crate::http::MeInfo, ()>>,
    fetched_at: Option<std::time::Instant>,
    gen: u64,
}

pub(crate) const CREDITS_TTL: std::time::Duration = std::time::Duration::from_secs(300);

impl CreditsCache {
    pub(crate) fn fresh() -> Self {
        Self {
            value: None,
            me: None,
            fetched_at: None,
            gen: 0,
        }
    }

    pub(crate) fn peek_credits(&self) -> String {
        match &self.value {
            Some(Ok(s)) => s.clone(),
            Some(Err(())) => "(unknown)".into(),
            None => "-".into(),
        }
    }

    pub(crate) fn peek_identity(&self) -> Option<crate::http::MeInfo> {
        match &self.me {
            Some(Ok(me)) => Some(me.clone()),
            _ => None,
        }
    }

    /// Refresh both credits and identity when stale.
    pub(crate) fn refresh(&mut self, base_url: &str, api_key: Option<&str>) {
        let expired = self
            .fetched_at
            .map(|t| t.elapsed() > CREDITS_TTL)
            .unwrap_or(true);
        if !expired {
            return;
        }
        match api_key {
            Some(key) => {
                // /v1/me carries email + username + balance in one call; fall
                // back to /v1/credits when it is unavailable (older gateway).
                self.me = Some(crate::http::fetch_me(base_url, key).map_err(|_| ()));
                let from_me = match &self.me {
                    Some(Ok(me)) => me.balance.map(crate::http::format_usd),
                    _ => None,
                };
                self.value = Some(match from_me {
                    Some(s) => Ok(s),
                    None => fetch_credits(base_url, key)
                        .map(|c| crate::http::format_usd(c["balance"].as_f64().unwrap_or(0.0)))
                        .map_err(|_| ()),
                });
            }
            None => {
                self.me = Some(Err(()));
                self.value = Some(Err(()));
            }
        }
        self.fetched_at = Some(std::time::Instant::now());
        self.gen = self.gen.saturating_add(1);
    }

    /// Return the cached credits display string, refreshing when stale.
    pub(crate) fn get(&mut self, base_url: &str, api_key: Option<&str>) -> String {
        self.refresh(base_url, api_key);
        self.peek_credits()
    }

    /// Cached identity, refreshing when stale. `None` when unknown.
    pub(crate) fn identity(
        &mut self,
        base_url: &str,
        api_key: Option<&str>,
    ) -> Option<crate::http::MeInfo> {
        self.refresh(base_url, api_key);
        self.peek_identity()
    }
}

#[cfg(feature = "native")]
pub(crate) fn kick_credits_refresh(
    cache: &Arc<Mutex<CreditsCache>>,
    base: String,
    key: Option<String>,
) {
    let Some(key) = key.filter(|s| !s.is_empty()) else {
        return;
    };
    let cache = cache.clone();
    let _ = std::thread::Builder::new()
        .name("anyr-credits".into())
        .spawn(move || {
            let mut tmp = CreditsCache::fresh();
            tmp.refresh(&base, Some(&key));
            if let Ok(mut slot) = cache.lock() {
                *slot = tmp;
            }
        });
}

#[cfg(feature = "native")]
pub(crate) fn patch_palette_header(state: &mut crate::tui::PaletteState, credits: &CreditsCache) {
    if let Some(me) = credits.peek_identity() {
        if let Some(line) = state.header.first_mut() {
            *line = format!("account  {}", me.display_label());
        }
    }
    let shown = credits.peek_credits();
    if let Some(line) = state.header.iter_mut().find(|l| l.starts_with("credits")) {
        *line = format!("credits  {shown}");
    }
}

pub(crate) fn catalog_lookup_enabled(env: &BTreeMap<String, String>) -> bool {
    match env.get("ANYR_NO_CATALOG").map(|s| s.as_str()) {
        Some("1" | "true" | "TRUE" | "yes") => false,
        _ => true,
    }
}

pub(crate) fn tool_command_for(path: &PathBuf, id: &str) -> String {
    let cfg = load_config_if_present(path);
    resolve_tool(cfg.as_ref(), id)
        .map(|t| t.command)
        .unwrap_or_else(|_| id.to_string())
}

pub(crate) fn persist_tool_command(path: &PathBuf, id: &str, command: &str) -> Result<(), String> {
    let mut cfg = load_config_if_present(path).unwrap_or_default();
    let mut tool = resolve_tool(Some(&cfg), id)?;
    tool.command = command.to_string();
    cfg.tools.insert(id.to_string(), tool);
    write_config(&cfg, path)
}

pub(crate) fn launcher_last_tool(
    path: &PathBuf,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> String {
    let cfg = load_config_if_present(path).unwrap_or_default();
    let profile = cfg.profiles.get(&cfg.active_profile);
    cfg.last_tool
        .clone()
        .or_else(|| profile.and_then(|p| p.default_tool.clone()))
        .or_else(|| get_string_flag(&parsed.flags, "tool"))
        .unwrap_or_else(|| {
            available_agents(env, |id| tool_command_for(path, id))
                .first()
                .map(|(id, _)| (*id).to_string())
                .unwrap_or_else(|| "claude".into())
        })
}

/// Bare `ar` / `anyr` opens the TUI on a terminal (or dump mode); pipes still get --help.
pub(crate) fn should_open_launcher(raw: &[String], interactive: bool, dump: bool) -> bool {
    raw.is_empty() && (interactive || dump)
}

#[cfg(test)]
mod tests {
    use super::should_open_launcher;

    #[test]
    fn bare_tty_opens_launcher_not_help() {
        assert!(should_open_launcher(&[], true, false));
        assert!(!should_open_launcher(&[], false, false));
        assert!(should_open_launcher(&[], false, true));
        assert!(!should_open_launcher(&["help".into()], true, false));
        assert!(!should_open_launcher(&["--help".into()], true, false));
        assert!(!should_open_launcher(&["config".into()], true, false));
    }
}
