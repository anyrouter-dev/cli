use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::auth::acquire_api_key;
use crate::config::{
    create_default_profile, resolve_config_path, set_active_profile, upsert_profile,
    valid_account_name, write_config, DefaultProfileInput, Profile, DEFAULT_PROFILE,
};
use crate::help::{command_help, resolve_bin, root_help, set_invoked_bin};
use crate::http::{
    create_key, delete_key, fetch_credits, fetch_keys, fetch_models, format_models_list,
    format_usage_report, is_active_key_row, most_used_model_id, reveal_key, validate_key,
    CatalogModel,
};
use crate::install::{
    agent_available, available_agents, ensure_tool_installed, missing_agents, KNOWN_AGENTS,
};
use crate::key::{
    load_config_if_present, mask_api_key, no_key_error, profile_for_agent, resolve_api_key,
    resolve_base_url, resolve_launch_api_key, resolve_launch_model,
};
use crate::parse::{get_string_flag, parse_cli_args, FlagValue, ParsedArgs};
use crate::spawn::{
    apply_routing_env, build_tool_env, canonical_tool, catalog_model_id, default_profile_for_env,
    display_model_id, effort_args_for, env_command_path, is_auto_model, model_args_for,
    normalize_effort, prepare_pi_wrapper, provider_args_for, render_dry_run, resolve_tool,
    session_model_label, spawn_child, BuildToolEnvInput,
};
use crate::term;
use crate::VERSION;

#[cfg(feature = "native")]
fn tui_wants_dump(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> bool {
    crate::tui::wants_dump(parsed, env)
}

#[cfg(not(feature = "native"))]
fn tui_wants_dump(_parsed: &ParsedArgs, _env: &BTreeMap<String, String>) -> bool {
    false
}

#[cfg(not(feature = "native"))]
fn tui_menu_select(
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
    fn new(
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
fn tui_palette_select(
    header: Vec<String>,
    entries: Vec<crate::tui::PaletteEntry>,
    on_idle: impl FnMut(&mut crate::tui::PaletteState),
) -> Result<Option<usize>, String> {
    crate::tui::run_palette_select_idle(header, entries, on_idle)
}

/// No fullscreen TUI (non-native build): the palette degrades to the inline
/// numbered-list picker — same entries, plain prompts.
#[cfg(feature = "native")]
fn pick_palette_action(
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
fn pick_palette_action(
    header: Vec<String>,
    entries: Vec<InlineEntry>,
    _cache: &Arc<Mutex<CreditsCache>>,
) -> Result<Option<usize>, String> {
    tui_palette_select(header, entries)
}

#[cfg(not(feature = "native"))]
fn tui_palette_select(
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

/// Compact HUD by default. Fullscreen palette only if ANYR_TUI=1.
#[cfg(feature = "native")]
fn launcher_uses_palette() -> bool {
    let tui = std::env::var("ANYR_TUI").unwrap_or_default();
    let t = tui.trim();
    (t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes"))
        && crate::tui::can_use_fullscreen()
}

#[cfg(not(feature = "native"))]
fn launcher_uses_palette() -> bool {
    false
}

#[cfg(feature = "native")]
fn tui_dump_palette(
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
fn tui_dump_palette(
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
fn tui_settings_select(
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
fn tui_dump_settings(state: crate::tui::SettingsState, env: &BTreeMap<String, String>) -> String {
    crate::tui::dump_settings(&state, crate::tui::dump_cols(env))
}

const LAUNCH_FLAGS: &[&str] = &[
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
enum CmdKind {
    Implemented,
    Stub,
    HelpOnly,
}

fn cmd_kind(command: &str) -> Option<CmdKind> {
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

fn known_command(command: &str) -> bool {
    cmd_kind(command).is_some()
}

fn canonical_command(command: &str) -> &str {
    match command {
        "update" => "upgrade",
        "implement" => "impl",
        other => canonical_tool(other),
    }
}

fn allowed_flags(command: &str) -> Option<&'static [&'static str]> {
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

fn assert_known_flags(
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

fn wants_help(parsed: &ParsedArgs) -> bool {
    parsed.flag_true("help")
        || parsed
            .passthrough
            .iter()
            .any(|a| a == "-h" || a == "--help")
}

fn help_topic(parsed: &ParsedArgs) -> String {
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

fn shift_passthrough(parsed: &ParsedArgs) -> ParsedArgs {
    let mut next = parsed.clone();
    if !next.passthrough.is_empty() {
        next.passthrough.remove(0);
    }
    next
}

fn run_auth(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let sub = parsed
        .passthrough
        .first()
        .map(String::as_str)
        .filter(|s| *s != "-h" && *s != "--help")
        .unwrap_or("");
    if sub.is_empty() {
        print!("{}", command_help("auth").unwrap_or_default());
        return Ok(0);
    }
    let rest = shift_passthrough(parsed);
    match sub {
        "login" | "setup" => run_login(&rest, env),
        "logout" => run_logout(&rest, env),
        "status" => run_whoami(&rest, env),
        "token" => run_auth_token(&rest, env),
        "switch" => run_auth_switch(&rest, env),
        other => Err(format!(
            "unknown command \"{other}\" for \"{} auth\"\n\n{}",
            crate::help::invoked_bin(),
            command_help("auth").unwrap_or_default()
        )),
    }
}

fn run_auth_token(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
    let name = get_string_flag(&parsed.flags, "profile")
        .or_else(|| env.get("ANYROUTER_PROFILE").cloned())
        .unwrap_or_else(|| cfg.active_profile.clone());
    let profile = cfg
        .profiles
        .get(&name)
        .ok_or_else(|| format!("Account \"{name}\" was not found."))?;
    let key = resolve_api_key(&parsed.flags, env, Some(profile)).ok_or_else(no_key_error)?;
    if parsed.flag_true("json") {
        let value = if parsed.flag_true("masked") {
            mask_api_key(Some(&key))
        } else {
            key.clone()
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "account": name,
                "token": value,
            }))
            .unwrap_or_else(|_| "{}".into())
        );
        return Ok(0);
    }
    if parsed.flag_true("masked") {
        println!("{}", mask_api_key(Some(&key)));
    } else {
        println!("{key}");
    }
    Ok(0)
}

fn run_auth_switch(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
    let names: Vec<String> = cfg.profiles.keys().cloned().collect();
    let name = parsed
        .passthrough
        .first()
        .cloned()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if !term::is_interactive() || names.is_empty() {
                return None;
            }
            let current = names.iter().position(|n| n == &cfg.active_profile);
            term::pick("Active account", &names, current)
                .ok()
                .map(|i| names[i].clone())
        })
        .ok_or_else(|| hint("Usage: {bin} auth switch <account>"))?;
    run_account_use(parsed, env, &name)
}

fn hint(template: &str) -> String {
    template.replace("{bin}", &crate::help::invoked_bin())
}

pub fn run(argv: Vec<String>, env: HashMap<String, String>) -> i32 {
    let raw = if argv.first().map(String::as_str) == Some("--") {
        argv[1..].to_vec()
    } else {
        argv
    };
    let env: BTreeMap<String, String> = env.into_iter().collect();
    #[cfg(not(target_arch = "wasm32"))]
    let argv0 = std::env::args().next();
    #[cfg(target_arch = "wasm32")]
    let argv0: Option<String> = None;
    set_invoked_bin(resolve_bin(
        argv0.as_deref(),
        env.get("ANYR_DISPLAY_BIN").map(String::as_str),
    ));

    let parsed = match parse_cli_args(&raw) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let command = parsed.command.as_str();
    if command == "--version" || command == "-v" {
        println!("{VERSION} (built {})", crate::buildinfo::display_time());
        return 0;
    }

    // parse_cli_args maps empty argv to command "help". Check emptiness first
    // so a real terminal gets the TUI launcher, not --help. Dump mode also
    // opens the launcher without a TTY (`ANYR_TUI_DUMP=1`).
    if should_open_launcher(&raw, term::is_interactive(), tui_wants_dump(&parsed, &env)) {
        #[cfg(feature = "native")]
        crate::upgrade::on_startup("menu", &parsed, &env);
        let empty = ParsedArgs {
            command: "menu".into(),
            flags: parsed.flags.clone(),
            passthrough: Vec::new(),
        };
        return match run_menu(&empty, &env) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("{err}");
                1
            }
        };
    }

    #[cfg(feature = "native")]
    crate::upgrade::on_startup(command, &parsed, &env);

    if raw.is_empty() || command == "help" || command == "--help" || command == "-h" {
        print!("{}", root_help());
        return 0;
    }
    if !known_command(command) {
        eprintln!(
            "Unknown command \"{command}\". Run \"{} --help\".",
            crate::help::invoked_bin()
        );
        return 1;
    }
    if wants_help(&parsed) {
        let topic = help_topic(&parsed);
        if let Some(help) = command_help(&topic).or_else(|| {
            if parsed.command == "auth" {
                command_help("auth")
            } else {
                None
            }
        }) {
            print!("{help}");
            return 0;
        }
    }
    if let Some(allowed) = allowed_flags(command) {
        if let Err(err) = assert_known_flags(command, &parsed.flags, allowed) {
            eprintln!("{err}");
            return 1;
        }
    }
    match dispatch(canonical_command(command), &parsed, &env) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn dispatch(
    command: &str,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    match command {
        "auth" => run_auth(parsed, env),
        "login" | "setup" => run_login(parsed, env),
        "logout" => run_logout(parsed, env),
        "models" => run_models(parsed, env),
        "usage" => run_usage(parsed, env),
        "whoami" | "status" => run_whoami(parsed, env),
        "config" => run_config(parsed, env),
        "account" => run_account(parsed, env),
        "keys" => run_keys(parsed, env),
        "menu" => run_menu(parsed, env),
        "claude" | "codex" | "grok" | "opencode" | "pool" | "pi" => {
            run_launch(command, parsed, env)
        }
        "relay" => {
            #[cfg(feature = "native")]
            {
                crate::relay::run(parsed, env)
            }
            #[cfg(not(feature = "native"))]
            {
                let _ = parsed;
                stub("relay")
            }
        }
        "cursor" | "cline" | "windsurf" => {
            print!("{}", command_help(command).unwrap_or_default());
            Ok(0)
        }
        "upgrade" | "update" => crate::upgrade::run(parsed, env),
        "onboard" | "impl" | "plan" | "fix" | "deploy" | "cp" => {
            crate::onboard::run(command, parsed)
        }
        _ => stub(command),
    }
}

fn stub(command: &str) -> Result<i32, String> {
    Err(format!(
        "\"{command}\" is not yet implemented in the native CLI. Run \"{} --help\" for available commands, or \"{} onboard\" for agent paste prompts.",
        crate::help::invoked_bin(),
        crate::help::invoked_bin(),
    ))
}

fn config_path(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> PathBuf {
    resolve_config_path(get_string_flag(&parsed.flags, "config").as_deref(), env)
}

fn persist_login(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    key: &str,
    source: &str,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let stored = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let base = resolve_base_url(&parsed.flags, stored);
    validate_key(&base, key)?;
    let key = resolve_latest_key(&base, key);
    let name = get_string_flag(&parsed.flags, "profile").unwrap_or_else(|| {
        existing
            .as_ref()
            .map(|c| c.active_profile.clone())
            .unwrap_or_else(|| DEFAULT_PROFILE.into())
    });
    let timeout = get_string_flag(&parsed.flags, "timeout").and_then(|s| s.parse().ok());
    let mut profile = create_default_profile(DefaultProfileInput {
        api_key: Some(key.clone()),
        base_url: Some(base.clone()),
        preset: get_string_flag(&parsed.flags, "preset"),
        timeout_ms: timeout,
        default_model: stored.and_then(|p| p.default_model.clone()),
    });
    // Clear legacy companion management keys; API keys with Key Management permission are enough.
    profile.management_key = None;
    if let Some(tool) = stored.and_then(|p| p.default_tool.clone()) {
        profile.default_tool = Some(tool);
    }
    if let Some(prev) = stored {
        profile.claude_haiku = prev.claude_haiku.clone();
        profile.claude_sonnet = prev.claude_sonnet.clone();
        profile.claude_opus = prev.claude_opus.clone();
        profile.claude_fable = prev.claude_fable.clone();
    }
    // Relay pairing and any unrecognized keys survive a relogin — re-pairing
    // the device (or losing unknown fields) on every login made credentials
    // feel like they were never persisted.
    if let Some(prev) = stored {
        profile.relay_token = prev.relay_token.clone();
        profile.relay_device_id = prev.relay_device_id.clone();
        profile.extra = prev.extra.clone();
    }
    let mut cfg = upsert_profile(existing.unwrap_or_default(), &name, profile);
    cfg.active_profile = name.clone();
    if !parsed.flag_true("yes") && term::is_interactive() {
        let models = models_for_picker(&base, Some(&key), env);
        if let Ok(id) = pick_model(&models, None, "Default model") {
            if let Some(p) = cfg.profiles.get_mut(&name) {
                set_model_slot(p, "default", id);
            }
        }
        let tools = ["claude", "codex", "grok", "opencode", "pi", "pool"];
        let labels: Vec<String> = tools
            .iter()
            .map(|t| {
                format!(
                    "{}  {}",
                    term::paint(term::tool_color(t), t),
                    crate::install::tool_hint(t)
                        .map(|h| h.label.to_string())
                        .unwrap_or_else(|| t.to_string())
                )
            })
            .collect();
        if let Ok(idx) = term::pick("Default coding agent", &labels, Some(0)) {
            if let Some(p) = cfg.profiles.get_mut(&name) {
                p.default_tool = Some(tools[idx].to_string());
            }
            cfg.last_tool = Some(tools[idx].to_string());
        }
    }
    write_config(&cfg, &path)?;
    println!(
        "{}  {}  {}",
        term::ok("Signed in."),
        term::dim(&format!("via {source}")),
        term::dim(&format!("key {}", mask_api_key(Some(&key))))
    );
    println!("{}  {}", term::dim("Saved"), path.display());
    Ok(0)
}

fn run_login(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let stored = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let base = resolve_base_url(&parsed.flags, stored);
    let acquired = acquire_api_key(&parsed.flags, env, &base, Some("cli"))?;
    persist_login(parsed, env, &acquired.api_key, &acquired.source)
}

fn model_pick_label(id: &str, models: &[CatalogModel]) -> String {
    if is_auto_model(id) {
        // The selectable preset id — never a "most used" ranking stand-in.
        return display_model_id(id);
    }
    if models
        .iter()
        .find(|m| catalog_model_id(&m.id) == id)
        .and_then(|m| m.context_length)
        .is_some_and(|n| n >= 1_000_000)
    {
        format!("{id}  ·  1M")
    } else {
        id.to_string()
    }
}

/// Picker catalog: pin `anyrouter/auto`, then real catalog ids in name order.
/// Usage-sorted fetch order is not the picker contents.
fn pick_ids(models: &[CatalogModel]) -> Vec<String> {
    let mut ids = vec!["anyrouter/auto".into()];
    let mut catalog: Vec<String> = models
        .iter()
        .map(|m| catalog_model_id(&m.id))
        .filter(|id| !id.is_empty() && !is_auto_model(id))
        .collect();
    catalog.sort();
    catalog.dedup();
    for id in catalog {
        if !ids.iter().any(|existing| existing == &id) {
            ids.push(id);
        }
    }
    ids
}

fn picker_choice_list(models: &[CatalogModel]) -> (Vec<String>, Vec<String>) {
    let ids = pick_ids(models);
    let labels = ids.iter().map(|id| model_pick_label(id, models)).collect();
    (ids, labels)
}

/// Catalog rows for the inline picker. Live fetch is optional: the preset
/// `anyrouter/auto` is always selectable even when lookup is off or fails.
fn models_for_picker(
    base: &str,
    key: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Vec<CatalogModel> {
    if !catalog_lookup_enabled(env) {
        return Vec::new();
    }
    fetch_models(base, key).unwrap_or_default()
}

fn render_model_picker_dump(
    models: &[CatalogModel],
    current: Option<&str>,
    env: &BTreeMap<String, String>,
) -> String {
    let (ids, labels) = picker_choice_list(models);
    let current_id = current.map(catalog_model_id);
    let shown_idx = current_id.and_then(|id| {
        ids.iter()
            .position(|s| s == &id || (is_auto_model(&id) && is_auto_model(s)))
    });
    #[cfg(feature = "native")]
    {
        crate::tui::dump_pick(
            "Default model",
            &labels,
            shown_idx,
            crate::tui::dump_cols(env),
        )
    }
    #[cfg(not(feature = "native"))]
    {
        let _ = (env, shown_idx);
        format!("{}\n", labels.join("\n"))
    }
}

fn pick_list(
    title: &str,
    header: &[String],
    items: &[String],
    current: Option<usize>,
) -> Result<usize, String> {
    #[cfg(feature = "native")]
    {
        crate::tui::pick_with_header(title, header, items, current)
    }
    #[cfg(not(feature = "native"))]
    {
        let _ = header;
        term::pick(title, items, current)
    }
}

fn pick_model(
    models: &[CatalogModel],
    current: Option<&str>,
    title: &str,
) -> Result<String, String> {
    let (ids, labels) = picker_choice_list(models);
    if ids.is_empty() {
        return Err("No models in catalog.".into());
    }
    let current_id = current.map(catalog_model_id);
    let shown_idx = current_id.and_then(|id| {
        ids.iter()
            .position(|s| s == &id || (is_auto_model(&id) && is_auto_model(s)))
    });
    let idx = pick_list(
        title,
        &["type to search · anyrouter/auto is pinned · enter to select".into()],
        &labels,
        shown_idx,
    )?;
    Ok(ids[idx].clone())
}

fn set_model_slot(profile: &mut Profile, slot: &str, id: String) {
    let id = catalog_model_id(&id);
    match slot {
        "haiku" => profile.claude_haiku = Some(id),
        "sonnet" => profile.claude_sonnet = Some(id),
        "opus" => profile.claude_opus = Some(id),
        "fable" => profile.claude_fable = Some(id),
        _ => {
            profile.default_model = if is_auto_model(&id) { None } else { Some(id) };
        }
    }
}

fn pick_claude_slot(profile: &Profile) -> Result<&'static str, String> {
    let items = vec![
        format!(
            "Default  ·  {}",
            session_model_label(profile.default_model())
        ),
        format!("Haiku    ·  {}", profile.claude_haiku()),
        format!("Sonnet   ·  {}", profile.claude_sonnet()),
        format!("Opus     ·  {}", profile.claude_opus()),
        format!("Fable    ·  {}", profile.claude_fable()),
    ];
    Ok(match term::pick("Which Claude model?", &items, Some(0))? {
        1 => "haiku",
        2 => "sonnet",
        3 => "opus",
        4 => "fable",
        _ => "default",
    })
}

fn slot_title(slot: &str) -> &'static str {
    match slot {
        "haiku" => "Haiku model",
        "sonnet" => "Sonnet model",
        "opus" => "Opus model",
        "fable" => "Fable model",
        _ => "Default model",
    }
}

fn slot_current<'a>(profile: &'a Profile, slot: &str) -> &'a str {
    match slot {
        "haiku" => profile.claude_haiku(),
        "sonnet" => profile.claude_sonnet(),
        "opus" => profile.claude_opus(),
        "fable" => profile.claude_fable(),
        _ => profile.default_model(),
    }
}

fn apply_claude_alias_flags(profile: &mut Profile, parsed: &ParsedArgs) -> bool {
    let mut changed = false;
    if let Some(v) = get_string_flag(&parsed.flags, "haiku") {
        profile.claude_haiku = Some(v);
        changed = true;
    }
    if let Some(v) = get_string_flag(&parsed.flags, "sonnet") {
        profile.claude_sonnet = Some(v);
        changed = true;
    }
    if let Some(v) = get_string_flag(&parsed.flags, "opus") {
        profile.claude_opus = Some(v);
        changed = true;
    }
    if let Some(v) = get_string_flag(&parsed.flags, "fable") {
        profile.claude_fable = Some(v);
        changed = true;
    }
    changed
}

fn known_model_id(models: &[CatalogModel], id: &str) -> bool {
    let id = catalog_model_id(id);
    is_auto_model(&id) || models.iter().any(|m| catalog_model_id(&m.id) == id)
}

fn save_model_slot(
    existing: Option<crate::config::Config>,
    path: &PathBuf,
    slot: &str,
    id: &str,
) -> Result<i32, String> {
    let mut cfg = existing.ok_or_else(no_key_error)?;
    let name = cfg.active_profile.clone();
    let Some(p) = cfg.profiles.get_mut(&name) else {
        return Err(no_key_error());
    };
    set_model_slot(p, slot, id.to_string());
    write_config(&cfg, path)?;
    let label = match slot {
        "haiku" => "haiku",
        "sonnet" => "sonnet",
        "opus" => "opus",
        "fable" => "fable",
        _ => "default model",
    };
    println!(
        "{}  {}  {}",
        term::ok("Saved"),
        label,
        term::model_id(&session_model_label(id))
    );
    Ok(0)
}

fn flag_agent(parsed: &ParsedArgs) -> Option<String> {
    get_string_flag(&parsed.flags, "agent").map(|s| canonical_tool(&s).to_string())
}

fn known_agent(name: &str) -> Result<String, String> {
    let id = canonical_tool(name);
    if KNOWN_AGENTS.iter().any(|(k, _)| *k == id) {
        Ok(id.to_string())
    } else {
        Err(format!(
            "Unknown coding agent \"{name}\". Known: claude, codex, grok, opencode, pi, pool."
        ))
    }
}

fn save_agent_model(path: &PathBuf, agent: &str, id: &str) -> Result<i32, String> {
    let agent = known_agent(agent)?;
    let mut cfg = load_config_if_present(path).ok_or_else(no_key_error)?;
    let id = catalog_model_id(id);
    let stored = if is_auto_model(&id) {
        None
    } else {
        Some(id.clone())
    };
    cfg.agent_binding_mut(&agent).default_model = stored;
    cfg.prune_empty_agents();
    write_config(&cfg, path)?;
    println!(
        "{}  {agent} model  {}",
        term::ok("Saved"),
        term::model_id(&session_model_label(&id))
    );
    Ok(0)
}

fn save_agent_account(path: &PathBuf, agent: &str, profile: &str) -> Result<i32, String> {
    let agent = known_agent(agent)?;
    let mut cfg = load_config_if_present(path).ok_or_else(no_key_error)?;
    if !cfg.profiles.contains_key(profile) {
        return Err(format!("Account \"{profile}\" was not found."));
    }
    cfg.agent_binding_mut(&agent).profile = Some(profile.to_string());
    write_config(&cfg, path)?;
    println!(
        "{}  {agent} account  {}",
        term::ok("Switched"),
        term::accent(profile)
    );
    Ok(0)
}

fn save_agent_key(path: &PathBuf, agent: &str, key: &str) -> Result<i32, String> {
    let agent = known_agent(agent)?;
    let mut cfg = load_config_if_present(path).ok_or_else(no_key_error)?;
    cfg.agent_binding_mut(&agent).api_key = Some(key.to_string());
    write_config(&cfg, path)?;
    println!(
        "{}  {agent} key  {}",
        term::ok("Switched"),
        mask_api_key(Some(key))
    );
    Ok(0)
}

fn palette_bind_detail(agent: &str) -> String {
    format!("for {agent}")
}

fn routing_toggle_detail(on: bool, agent: &str) -> String {
    format!("{} · for {agent}", if on { "on" } else { "off" })
}

fn toggle_agent_routing_field(
    path: &std::path::Path,
    agent: &str,
    field: RoutingField,
) -> Result<i32, String> {
    let agent = known_agent(agent)?;
    let mut cfg = load_config_if_present(path).unwrap_or_default();
    let routing = &mut cfg.agent_binding_mut(&agent).routing;
    let on = match field {
        RoutingField::Exacto => {
            routing.set_exacto(!routing.wants_exacto());
            routing.wants_exacto()
        }
        RoutingField::Tools => {
            routing.set_require_tools(!routing.requires_tools());
            routing.requires_tools()
        }
        RoutingField::MinContext => {
            routing.set_require_1m(!routing.requires_1m_context());
            routing.requires_1m_context()
        }
    };
    cfg.prune_empty_agents();
    write_config(&cfg, path)?;
    println!(
        "{}  {agent} {} {}",
        term::ok("Saved"),
        field.label(),
        if on { "on" } else { "off" }
    );
    Ok(0)
}

fn run_models(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let profile = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let sub = parsed.passthrough.first().map(String::as_str);
    let agent = flag_agent(parsed);
    // Pinning a caller-supplied id to an agent does not need the catalog —
    // do not invent ids; the user (or TUI picker) already chose one.
    if let Some(agent) = agent.as_deref() {
        let positional = parsed.passthrough.get(1).cloned().filter(|s| !s.is_empty());
        if sub == Some("use") {
            if let Some(id) = positional {
                return save_agent_model(&path, agent, &id);
            }
        }
    }
    let key = resolve_api_key(&parsed.flags, env, profile);
    let base = resolve_base_url(&parsed.flags, profile);
    let flag_slots = [
        ("haiku", get_string_flag(&parsed.flags, "haiku")),
        ("sonnet", get_string_flag(&parsed.flags, "sonnet")),
        ("opus", get_string_flag(&parsed.flags, "opus")),
        ("fable", get_string_flag(&parsed.flags, "fable")),
    ];
    let has_alias_flags = flag_slots.iter().any(|(_, v)| v.is_some());
    if tui_wants_dump(parsed, env) {
        let models = models_for_picker(&base, key.as_deref(), env);
        let current = profile.map(|p| slot_current(p, "default").to_string());
        print!(
            "{}",
            render_model_picker_dump(&models, current.as_deref(), env)
        );
        return Ok(0);
    }
    // `anyrouter/auto` is a first-class preset — persist it without a catalog fetch.
    if sub == Some("use") && !has_alias_flags {
        let positional = parsed.passthrough.get(1).cloned().filter(|s| !s.is_empty());
        if let Some(id) = positional {
            if is_auto_model(&id) {
                return save_model_slot(existing, &path, "default", &display_model_id(&id));
            }
        }
    }
    if parsed.flag_true("pick") && term::is_interactive() {
        let models = models_for_picker(&base, key.as_deref(), env);
        if let Some(agent) = agent.as_deref() {
            let current = existing
                .as_ref()
                .and_then(|c| c.agent_binding(agent))
                .and_then(|b| b.default_model.clone())
                .or_else(|| profile.map(|p| p.default_model().to_string()));
            let id = pick_model(&models, current.as_deref(), &format!("{agent} model"))?;
            return save_agent_model(&path, agent, &id);
        }
        let slot = profile
            .map(pick_claude_slot)
            .transpose()?
            .unwrap_or("default");
        let current = profile.map(|p| slot_current(p, slot).to_string());
        let id = pick_model(&models, current.as_deref(), slot_title(slot))?;
        return save_model_slot(existing, &path, slot, &catalog_model_id(&id));
    }
    // `models use --agent` with no id: same picker as `--pick`, no catalog required.
    if sub == Some("use") && !has_alias_flags {
        if let Some(agent) = agent.as_deref() {
            if term::is_interactive() {
                let models = models_for_picker(&base, key.as_deref(), env);
                let current = existing
                    .as_ref()
                    .and_then(|c| c.agent_binding(agent))
                    .and_then(|b| b.default_model.clone())
                    .or_else(|| profile.map(|p| p.default_model().to_string()));
                let id = pick_model(&models, current.as_deref(), &format!("{agent} model"))?;
                return save_agent_model(&path, agent, &id);
            }
        }
    }
    let models = fetch_models(&base, key.as_deref())?;
    if sub == Some("use") || has_alias_flags {
        if agent.is_some() {
            return Err(hint(
                "Usage: {bin} models use <id> --agent <claude|codex|grok|opencode|pi|pool>",
            ));
        }
        let mut assigned = false;
        for (slot, value) in &flag_slots {
            let Some(id) = value.clone() else {
                continue;
            };
            if !known_model_id(&models, &id) {
                return Err(hint(&format!(
                    "Unknown model \"{id}\". Run: {{bin}} models"
                )));
            }
            save_model_slot(existing.clone(), &path, slot, &catalog_model_id(&id))?;
            assigned = true;
        }
        let positional = parsed.passthrough.get(1).cloned().filter(|s| !s.is_empty());
        if let Some(id) = positional {
            if !known_model_id(&models, &id) {
                return Err(hint(&format!(
                    "Unknown model \"{id}\". Run: {{bin}} models"
                )));
            }
            save_model_slot(
                load_config_if_present(&path),
                &path,
                "default",
                &catalog_model_id(&id),
            )?;
            assigned = true;
        }
        if assigned {
            return Ok(0);
        }
        let id = if term::is_interactive() {
            let slot = profile
                .map(pick_claude_slot)
                .transpose()?
                .unwrap_or("default");
            let current = profile.map(|p| slot_current(p, slot).to_string());
            pick_model(&models, current.as_deref(), slot_title(slot)).map(|id| (slot, id))
        } else {
            Err(hint(
                "Usage: {bin} models use <id>   or   {bin} models use --haiku|--sonnet|--opus <id>",
            ))
        }?;
        if !known_model_id(&models, &id.1) {
            return Err(hint(&format!(
                "Unknown model \"{}\". Run: {{bin}} models",
                id.1
            )));
        }
        return save_model_slot(
            load_config_if_present(&path),
            &path,
            id.0,
            &catalog_model_id(&id.1),
        );
    }
    let pinned = profile
        .map(|p| display_model_id(p.default_model()))
        .into_iter()
        .collect::<Vec<_>>();
    let preset = profile.map(|p| p.pinned_preset().to_string());
    let (stdout, _) = format_models_list(
        &models,
        &pinned,
        preset.as_deref(),
        parsed.flag_true("json"),
    );
    print!("{stdout}");
    Ok(0)
}

fn run_usage(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let profile = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let key = resolve_api_key(&parsed.flags, env, profile).ok_or_else(no_key_error)?;
    let base = resolve_base_url(&parsed.flags, profile);
    let credits = fetch_credits(&base, &key)?;
    print!(
        "{}",
        format_usage_report(&credits, parsed.flag_true("json"))
    );
    Ok(0)
}

fn run_whoami(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let Some(cfg) = load_config_if_present(&path) else {
        return Err(no_key_error());
    };
    let name = get_string_flag(&parsed.flags, "profile")
        .or_else(|| env.get("ANYROUTER_PROFILE").cloned())
        .unwrap_or_else(|| cfg.active_profile.clone());
    let profile = cfg
        .profiles
        .get(&name)
        .ok_or_else(|| format!("Profile \"{name}\" was not found in AnyRouter config."))?;
    let key = resolve_api_key(&parsed.flags, env, Some(profile));
    if parsed.flag_true("json") {
        let payload = serde_json::json!({
            "active_account": name,
            "config": path.display().to_string(),
            "api_key": mask_api_key(key.as_deref()),
            "default_model": display_model_id(profile.default_model()),
            "claude_haiku": profile.claude_haiku(),
            "claude_sonnet": profile.claude_sonnet(),
            "claude_opus": profile.claude_opus(),
            "claude_fable": profile.claude_fable(),
            "default_tool": profile.default_tool,
            "base_url": profile.base_url(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
        );
        return Ok(0);
    }
    println!("{}  {}", term::dim("active account"), term::accent(&name));
    println!("{}  {}", term::dim("config         "), path.display());
    println!(
        "{}  {}",
        term::dim("api_key        "),
        mask_api_key(key.as_deref())
    );
    println!(
        "{}  {}",
        term::dim("default_model  "),
        term::model_id(&session_model_label(profile.default_model()))
    );
    println!(
        "{}  {}",
        term::dim("claude_haiku   "),
        term::model_id(profile.claude_haiku())
    );
    println!(
        "{}  {}",
        term::dim("claude_sonnet  "),
        term::model_id(profile.claude_sonnet())
    );
    println!(
        "{}  {}",
        term::dim("claude_opus    "),
        term::model_id(profile.claude_opus())
    );
    if let Some(tool) = &profile.default_tool {
        println!(
            "{}  {}",
            term::dim("default_tool   "),
            term::paint(term::tool_color(tool), tool)
        );
    }
    Ok(0)
}

fn print_config_status(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
) -> Result<(), String> {
    let cfg = load_config_if_present(path).unwrap_or_default();
    let profile = cfg.profiles.get(&cfg.active_profile);
    let key = resolve_api_key(&parsed.flags, env, profile);
    let base = resolve_base_url(&parsed.flags, profile);
    term::print_brand_header(&[
        &term::bold("AnyRouter config"),
        &format!(
            "{}  {}",
            term::dim("account"),
            term::accent(&cfg.active_profile)
        ),
        &format!(
            "{}  {}",
            term::dim("api_key "),
            mask_api_key(key.as_deref())
        ),
    ]);
    println!(
        "{}  {}",
        term::dim("model   "),
        term::model_id(&session_model_label(
            profile.map(|p| p.default_model()).unwrap_or("auto"),
        ))
    );
    if let Some(p) = profile {
        println!(
            "{}  {}",
            term::dim("haiku   "),
            term::model_id(p.claude_haiku())
        );
        println!(
            "{}  {}",
            term::dim("sonnet  "),
            term::model_id(p.claude_sonnet())
        );
        println!(
            "{}  {}",
            term::dim("opus    "),
            term::model_id(p.claude_opus())
        );
        println!(
            "{}  {}",
            term::dim("fable   "),
            term::model_id(p.claude_fable())
        );
    }
    if let Some(tool) = profile.and_then(|p| p.default_tool.as_deref()) {
        println!(
            "{}  {}",
            term::dim("agent   "),
            term::paint(term::tool_color(tool), tool)
        );
    }
    println!("{}  {}", term::dim("file    "), path.display());
    if let Some(key) = key.as_deref() {
        if let Ok(credits) = fetch_credits(&base, key) {
            print!("{}", format_usage_report(&credits, false));
        }
    }
    Ok(())
}

/// Which edit a focused settings row triggers.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingKind {
    Account,
    ApiKey,
    /// Model slot: "default", "haiku", "sonnet", "opus", "fable".
    Model(&'static str),
    Agent,
    AutoUpdate,
    Channel,
    Install(&'static str),
    ToolCommand(&'static str),
    GatewayDiscovery,
    /// Read-only mapping row.
    Mapping,
    AgentAccount(&'static str),
    AgentKey(&'static str),
    AgentModel(&'static str),
    AgentRouting(&'static str, RoutingField),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutingField {
    Exacto,
    Tools,
    MinContext,
}

impl RoutingField {
    fn label(self) -> &'static str {
        match self {
            RoutingField::Exacto => "exacto",
            RoutingField::Tools => "tools",
            RoutingField::MinContext => "1M ctx",
        }
    }

    #[allow(dead_code)]
    fn action_kind(self) -> &'static str {
        match self {
            RoutingField::Exacto => "exacto",
            RoutingField::Tools => "tools",
            RoutingField::MinContext => "1m",
        }
    }
}

fn run_config_tui(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    if !tui_wants_dump(parsed, env) && stored_api_key(parsed, env, &path).is_none() {
        run_login(parsed, env)?;
        if stored_api_key(parsed, env, &path).is_none() {
            return Err(hint(
                "Not signed in. Run `{bin} auth login` or pass --key / ANYROUTER_API_KEY.",
            ));
        }
    }
    #[cfg(feature = "native")]
    {
        if tui_wants_dump(parsed, env) {
            let (state, _) =
                config_settings_frame(parsed, env, &path, false, &mut CreditsCache::fresh(), 0);
            print!("{}", tui_dump_settings(state, env));
            return Ok(0);
        }
        config_settings_loop(parsed, env, &path)
    }
    #[cfg(not(feature = "native"))]
    config_menu_loop_legacy(parsed, env, &path)
}

/// Build one settings frame: grouped rows with current values, plus a parallel
/// list mapping row index → edit kind. `online` gates network (dump stays
/// offline-deterministic).
#[cfg(feature = "native")]
fn settings_tab_names() -> Vec<String> {
    let mut tabs = vec!["general".to_string()];
    tabs.extend(KNOWN_AGENTS.iter().map(|(id, _)| (*id).to_string()));
    tabs
}

fn tool_command_for(path: &PathBuf, id: &str) -> String {
    let cfg = load_config_if_present(path);
    resolve_tool(cfg.as_ref(), id)
        .map(|t| t.command)
        .unwrap_or_else(|_| id.to_string())
}

#[cfg(feature = "native")]
fn config_settings_frame(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
    online: bool,
    cache: &mut CreditsCache,
    tab: usize,
) -> (crate::tui::SettingsState, Vec<Option<SettingKind>>) {
    use crate::tui::{SettingRow, SettingsState, Tone};

    let cfg = load_config_if_present(path).unwrap_or_default();
    let profile = cfg.profiles.get(&cfg.active_profile);
    let key = stored_api_key(parsed, env, path);
    let base = resolve_base_url(&parsed.flags, profile);
    let signed_in = key.is_some();

    let identity = if online {
        cache.identity(&base, key.as_deref())
    } else {
        None
    };
    let account_value = match (&identity, signed_in) {
        (Some(me), _) => me.display_label(),
        (None, true) => cfg.active_profile.clone(),
        (None, false) => "(not signed in)".into(),
    };
    let account_tone = if signed_in { Tone::Normal } else { Tone::Warn };
    let credits_value = if !online {
        "-".into()
    } else {
        match identity.as_ref().and_then(|me| me.balance) {
            Some(b) => crate::http::format_usd(b),
            None => cache.get(&base, key.as_deref()),
        }
    };

    let header = vec![
        format!("account  {account_value}"),
        format!("credits  {credits_value}"),
        format!("file     {}", path.display()),
    ];

    let mut rows: Vec<SettingRow> = Vec::new();
    let mut kinds: Vec<Option<SettingKind>> = Vec::new();

    let tabs = settings_tab_names();
    let tab = tab.min(tabs.len().saturating_sub(1));
    if tab == 0 {
        fill_general_settings(
            &mut rows,
            &mut kinds,
            path,
            parsed,
            env,
            profile,
            signed_in,
            key.as_deref(),
            account_value.clone(),
            account_tone,
            &cfg,
        );
    } else if let Some((id, label)) = KNOWN_AGENTS.get(tab - 1).copied() {
        fill_agent_settings(&mut rows, &mut kinds, path, env, profile, id, label);
    }

    (
        SettingsState::new("Config", header, rows).with_tabs(tabs, tab),
        kinds,
    )
}

#[cfg(feature = "native")]
#[allow(clippy::too_many_arguments)]
fn fill_general_settings(
    rows: &mut Vec<crate::tui::SettingRow>,
    kinds: &mut Vec<Option<SettingKind>>,
    path: &PathBuf,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    profile: Option<&Profile>,
    signed_in: bool,
    key: Option<&str>,
    account_value: String,
    account_tone: crate::tui::Tone,
    cfg: &crate::config::Config,
) {
    use crate::tui::{SettingRow, Tone};
    fn section(rows: &mut Vec<SettingRow>, kinds: &mut Vec<Option<SettingKind>>, name: &str) {
        if !rows.is_empty() {
            rows.push(SettingRow::Gap);
            kinds.push(None);
        }
        rows.push(SettingRow::Section(name.into()));
        kinds.push(None);
    }
    fn entry(
        rows: &mut Vec<SettingRow>,
        kinds: &mut Vec<Option<SettingKind>>,
        label: &str,
        value: String,
        tone: Tone,
        kind: SettingKind,
    ) {
        rows.push(SettingRow::Entry {
            label: label.into(),
            value,
            tone,
        });
        kinds.push(Some(kind));
    }

    section(rows, kinds, "Account");
    entry(
        rows,
        kinds,
        "account",
        account_value,
        account_tone,
        SettingKind::Account,
    );
    let key_value = if signed_in {
        mask_api_key(key)
    } else {
        "(not set)".into()
    };
    entry(
        rows,
        kinds,
        "api key",
        key_value,
        if signed_in { Tone::Normal } else { Tone::Muted },
        SettingKind::ApiKey,
    );

    section(rows, kinds, "Model");
    entry(
        rows,
        kinds,
        "default",
        session_model_label(profile.map(|p| p.default_model()).unwrap_or("auto")),
        Tone::Model,
        SettingKind::Model("default"),
    );

    section(rows, kinds, "Agent");
    let agent = launcher_last_tool(path, parsed, env);
    let agent_pinned = profile.and_then(|p| p.default_tool.clone()).is_some();
    entry(
        rows,
        kinds,
        "coding agent",
        agent,
        if agent_pinned {
            Tone::Normal
        } else {
            Tone::Muted
        },
        SettingKind::Agent,
    );
    let present = available_agents(env, |id| tool_command_for(path, id));
    entry(
        rows,
        kinds,
        "on PATH",
        if present.is_empty() {
            "none detected".into()
        } else {
            present
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>()
                .join(", ")
        },
        if present.is_empty() {
            Tone::Warn
        } else {
            Tone::Good
        },
        SettingKind::Mapping,
    );

    section(rows, kinds, "General");
    entry(
        rows,
        kinds,
        "auto-update",
        if cfg.auto_update() {
            "enabled".into()
        } else {
            "disabled".into()
        },
        if cfg.auto_update() {
            Tone::Good
        } else {
            Tone::Muted
        },
        SettingKind::AutoUpdate,
    );
    entry(
        rows,
        kinds,
        "update channel",
        cfg.channel().into(),
        Tone::Normal,
        SettingKind::Channel,
    );
}

#[cfg(feature = "native")]
fn fill_agent_settings(
    rows: &mut Vec<crate::tui::SettingRow>,
    kinds: &mut Vec<Option<SettingKind>>,
    path: &PathBuf,
    env: &BTreeMap<String, String>,
    profile: Option<&Profile>,
    id: &'static str,
    label: &str,
) {
    use crate::tui::{SettingRow, Tone};
    fn section(rows: &mut Vec<SettingRow>, kinds: &mut Vec<Option<SettingKind>>, name: &str) {
        if !rows.is_empty() {
            rows.push(SettingRow::Gap);
            kinds.push(None);
        }
        rows.push(SettingRow::Section(name.into()));
        kinds.push(None);
    }
    fn entry(
        rows: &mut Vec<SettingRow>,
        kinds: &mut Vec<Option<SettingKind>>,
        label: &str,
        value: String,
        tone: Tone,
        kind: SettingKind,
    ) {
        rows.push(SettingRow::Entry {
            label: label.into(),
            value,
            tone,
        });
        kinds.push(Some(kind));
    }

    let cfg = load_config_if_present(path);
    let tool = resolve_tool(cfg.as_ref(), id)
        .unwrap_or_else(|_| crate::spawn::resolve_tool(None, id).expect("known agent"));
    let present = agent_available(id, &tool.command, env);

    section(rows, kinds, label);
    entry(
        rows,
        kinds,
        "status",
        if present {
            "on PATH".into()
        } else {
            "not installed".into()
        },
        if present { Tone::Good } else { Tone::Warn },
        SettingKind::Install(id),
    );
    entry(
        rows,
        kinds,
        "command",
        tool.command.clone(),
        Tone::Normal,
        SettingKind::ToolCommand(id),
    );
    let hint = crate::install::tool_hint(id);
    entry(
        rows,
        kinds,
        "install",
        if present {
            "reinstall / update".into()
        } else {
            hint.map(|h| h.install.to_string())
                .unwrap_or_else(|| "install".into())
        },
        if present { Tone::Muted } else { Tone::Warn },
        SettingKind::Install(id),
    );

    let binding = cfg.as_ref().and_then(|c| c.agent_binding(id));
    section(rows, kinds, "Bindings");
    let account = binding
        .and_then(|b| b.profile.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "session default".into());
    entry(
        rows,
        kinds,
        "account",
        account,
        if binding.and_then(|b| b.profile.as_deref()).is_some() {
            Tone::Normal
        } else {
            Tone::Muted
        },
        SettingKind::AgentAccount(id),
    );
    let key_value = binding
        .and_then(|b| b.api_key.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|k| mask_api_key(Some(k)))
        .unwrap_or_else(|| "profile default".into());
    entry(
        rows,
        kinds,
        "api key",
        key_value,
        if binding.and_then(|b| b.api_key.as_deref()).is_some() {
            Tone::Normal
        } else {
            Tone::Muted
        },
        SettingKind::AgentKey(id),
    );
    let model_value = binding
        .and_then(|b| b.default_model.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(session_model_label)
        .unwrap_or_else(|| "session default".into());
    entry(
        rows,
        kinds,
        "model",
        model_value,
        if binding.and_then(|b| b.default_model.as_deref()).is_some() {
            Tone::Model
        } else {
            Tone::Muted
        },
        SettingKind::AgentModel(id),
    );

    section(rows, kinds, "Routing");
    let routing = binding.map(|b| b.routing.clone()).unwrap_or_default();
    entry(
        rows,
        kinds,
        "exacto",
        if routing.wants_exacto() {
            "on".into()
        } else {
            "off".into()
        },
        if routing.wants_exacto() {
            Tone::Good
        } else {
            Tone::Muted
        },
        SettingKind::AgentRouting(id, RoutingField::Exacto),
    );
    entry(
        rows,
        kinds,
        "tools",
        if routing.requires_tools() {
            "on".into()
        } else {
            "off".into()
        },
        if routing.requires_tools() {
            Tone::Good
        } else {
            Tone::Muted
        },
        SettingKind::AgentRouting(id, RoutingField::Tools),
    );
    entry(
        rows,
        kinds,
        "1M ctx",
        if routing.requires_1m_context() {
            "on".into()
        } else {
            "off".into()
        },
        if routing.requires_1m_context() {
            Tone::Good
        } else {
            Tone::Muted
        },
        SettingKind::AgentRouting(id, RoutingField::MinContext),
    );

    section(rows, kinds, "Mapping");
    entry(
        rows,
        kinds,
        "base URL env",
        tool.base_url_env.clone(),
        Tone::Muted,
        SettingKind::Mapping,
    );
    entry(
        rows,
        kinds,
        "auth env",
        tool.auth_env.clone(),
        Tone::Muted,
        SettingKind::Mapping,
    );
    entry(
        rows,
        kinds,
        "model env",
        tool.model_env.clone().unwrap_or_else(|| "(none)".into()),
        Tone::Muted,
        SettingKind::Mapping,
    );
    entry(
        rows,
        kinds,
        "URL suffix",
        if tool.base_suffix.is_empty() {
            "(none)".into()
        } else {
            tool.base_suffix.clone()
        },
        Tone::Muted,
        SettingKind::Mapping,
    );

    if id == "claude" {
        section(rows, kinds, "Claude aliases");
        for (slot_label, slot) in [
            ("haiku", "haiku"),
            ("sonnet", "sonnet"),
            ("opus", "opus"),
            ("fable", "fable"),
        ] {
            let pinned = match slot {
                "haiku" => nonempty_slot(&profile.and_then(|p| p.claude_haiku.clone())),
                "sonnet" => nonempty_slot(&profile.and_then(|p| p.claude_sonnet.clone())),
                "opus" => nonempty_slot(&profile.and_then(|p| p.claude_opus.clone())),
                _ => nonempty_slot(&profile.and_then(|p| p.claude_fable.clone())),
            };
            let value = match &pinned {
                Some(mid) => catalog_model_id(mid),
                None => format!("{} · default", slot_current_opt(profile, slot)),
            };
            let tone = if pinned.is_some() {
                Tone::Model
            } else {
                Tone::Muted
            };
            let slot_static: &'static str = match slot {
                "haiku" => "haiku",
                "sonnet" => "sonnet",
                "opus" => "opus",
                _ => "fable",
            };
            entry(
                rows,
                kinds,
                slot_label,
                value,
                tone,
                SettingKind::Model(slot_static),
            );
        }
        entry(
            rows,
            kinds,
            "gateway discovery",
            if tool.enable_gateway_model_discovery {
                "on".into()
            } else {
                "off".into()
            },
            if tool.enable_gateway_model_discovery {
                Tone::Good
            } else {
                Tone::Muted
            },
            SettingKind::GatewayDiscovery,
        );
    }
}

#[cfg_attr(not(feature = "native"), allow(dead_code))]
fn nonempty_slot(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Effective slot value for display when nothing is pinned.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
fn slot_current_opt(profile: Option<&Profile>, slot: &str) -> String {
    slot_current(profile.unwrap_or(&Profile::default()), slot).to_string()
}

/// Settings loop: render → edit/reset → re-render with fresh values.
#[cfg(feature = "native")]
fn config_settings_loop(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
) -> Result<i32, String> {
    let mut cache = CreditsCache::fresh();
    let mut tab = 0usize;
    let n_tabs = settings_tab_names().len().max(1);
    loop {
        let (state, kinds) = config_settings_frame(parsed, env, path, true, &mut cache, tab);
        let Some(outcome) = tui_settings_select(state)? else {
            return Ok(0);
        };
        let result = match outcome {
            crate::tui::SettingsOutcome::Edit(idx) => kinds
                .get(idx)
                .copied()
                .flatten()
                .map(|kind| config_edit_row(parsed, env, path, kind)),
            crate::tui::SettingsOutcome::Reset(idx) => kinds
                .get(idx)
                .copied()
                .flatten()
                .map(|kind| config_reset_row(path, kind)),
            crate::tui::SettingsOutcome::NextTab => {
                tab = (tab + 1) % n_tabs;
                None
            }
            crate::tui::SettingsOutcome::PrevTab => {
                tab = (tab + n_tabs - 1) % n_tabs;
                None
            }
            crate::tui::SettingsOutcome::GotoTab(i) => {
                tab = i.min(n_tabs.saturating_sub(1));
                None
            }
            crate::tui::SettingsOutcome::Close | crate::tui::SettingsOutcome::Stay => None,
        };
        if let Some(Err(err)) = result {
            if err != "Cancelled." {
                eprintln!("{}", term::err(&err));
            }
        }
    }
}

#[cfg_attr(not(feature = "native"), allow(dead_code))]
fn config_edit_row(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
    kind: SettingKind,
) -> Result<i32, String> {
    match kind {
        SettingKind::Account => config_account_actions(parsed, env),
        SettingKind::ApiKey => {
            let mut next = parsed.clone();
            next.command = "keys".into();
            next.passthrough = vec!["use".into()];
            match run_keys(&next, env) {
                Err(err) if err == "Cancelled." => Ok(0),
                other => other,
            }
        }
        SettingKind::Model(slot) => {
            let existing = load_config_if_present(path);
            let profile = existing
                .as_ref()
                .and_then(|c| c.profiles.get(&c.active_profile));
            let key = resolve_api_key(&parsed.flags, env, profile);
            let base = resolve_base_url(&parsed.flags, profile);
            let models = models_for_picker(&base, key.as_deref(), env);
            let current = profile.map(|p| slot_current(p, slot).to_string());
            let id = pick_model(&models, current.as_deref(), slot_title(slot))?;
            save_model_slot(existing, path, slot, &id)
        }
        SettingKind::Agent => {
            let last = launcher_last_tool(path, parsed, env);
            let labels: Vec<String> = KNOWN_AGENTS
                .iter()
                .map(|(id, label)| format!("{id}  —  {label}"))
                .collect();
            let current = KNOWN_AGENTS.iter().position(|(id, _)| *id == last.as_str());
            let idx = term::pick("Coding agent", &labels, current)?;
            let (tool, label) = KNOWN_AGENTS[idx];
            let mut cfg = load_config_if_present(path).unwrap_or_default();
            if let Some(p) = cfg.profiles.get_mut(&cfg.active_profile) {
                p.default_tool = Some(tool.into());
            }
            write_config(&cfg, path)?;
            println!(
                "{}  coding agent  {}",
                term::ok("Saved"),
                term::paint(term::tool_color(tool), label)
            );
            Ok(0)
        }
        SettingKind::AutoUpdate => {
            let mut cfg = load_config_if_present(path).unwrap_or_default();
            cfg.auto_update = Some(!cfg.auto_update());
            write_config(&cfg, path)?;
            println!(
                "{}  auto-update {}",
                term::ok("Saved"),
                if cfg.auto_update() {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            Ok(0)
        }
        SettingKind::Install(id) => {
            let command = tool_command_for(path, id);
            match ensure_tool_installed(id, &command, true) {
                Ok(resolved) => {
                    persist_tool_command(path, id, &resolved)?;
                    println!(
                        "{}  {}  {}",
                        term::ok("Installed"),
                        id,
                        term::dim(&resolved)
                    );
                    Ok(0)
                }
                Err(err) => Err(err),
            }
        }
        SettingKind::ToolCommand(id) => {
            let current = tool_command_for(path, id);
            let next = term::prompt(&format!("Command path for {id} (Enter keeps {current}): "))?;
            let next = next.trim();
            if next.is_empty() {
                return Ok(0);
            }
            let mut cfg = load_config_if_present(path).unwrap_or_default();
            let mut tool = resolve_tool(Some(&cfg), id)?;
            tool.command = next.to_string();
            cfg.tools.insert(id.to_string(), tool);
            write_config(&cfg, path)?;
            println!("{}  {id} command  {next}", term::ok("Saved"));
            Ok(0)
        }
        SettingKind::GatewayDiscovery => {
            let mut cfg = load_config_if_present(path).unwrap_or_default();
            let mut tool = resolve_tool(Some(&cfg), "claude")?;
            tool.enable_gateway_model_discovery = !tool.enable_gateway_model_discovery;
            let on = tool.enable_gateway_model_discovery;
            cfg.tools.insert("claude".into(), tool);
            write_config(&cfg, path)?;
            println!(
                "{}  gateway discovery  {}",
                term::ok("Saved"),
                if on { "on" } else { "off" }
            );
            Ok(0)
        }
        SettingKind::Mapping => Ok(0),
        SettingKind::AgentAccount(id) => {
            let mut next = parsed.clone();
            next.flags
                .insert("agent".into(), FlagValue::Value(id.to_string()));
            config_account_actions(&next, env)
        }
        SettingKind::AgentKey(id) => {
            let mut next = parsed.clone();
            next.command = "keys".into();
            next.passthrough = vec!["use".into()];
            next.flags
                .insert("agent".into(), FlagValue::Value(id.to_string()));
            match run_keys(&next, env) {
                Err(err) if err == "Cancelled." => Ok(0),
                other => other,
            }
        }
        SettingKind::AgentModel(id) => {
            let mut next = parsed.clone();
            next.command = "models".into();
            next.flags.insert("pick".into(), FlagValue::Bool(true));
            next.flags
                .insert("agent".into(), FlagValue::Value(id.to_string()));
            match run_models(&next, env) {
                Err(err) if err == "Cancelled." => Ok(0),
                other => other,
            }
        }
        SettingKind::AgentRouting(id, field) => toggle_agent_routing_field(path, id, field),
        SettingKind::Channel => {
            let choices = ["stable", "beta"];
            let current = choices.iter().position(|c| *c == cfg_channel(path));
            let idx = term::pick(
                "Update channel",
                &choices.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
                current,
            )?;
            let mut cfg = load_config_if_present(path).unwrap_or_default();
            cfg.channel = Some(choices[idx].into());
            write_config(&cfg, path)?;
            println!("{}  channel  {}", term::ok("Saved"), choices[idx]);
            Ok(0)
        }
    }
}

#[cfg_attr(not(feature = "native"), allow(dead_code))]
fn cfg_channel(path: &std::path::Path) -> String {
    load_config_if_present(path)
        .map(|c| c.channel().to_string())
        .unwrap_or_else(|| "stable".into())
}

#[cfg_attr(not(feature = "native"), allow(dead_code))]
fn config_account_actions(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let cfg = load_config_if_present(&path).unwrap_or_default();
    let mut names: Vec<String> = cfg.profiles.keys().cloned().collect();
    names.sort();
    let mut labels: Vec<String> = names
        .iter()
        .map(|n| {
            let p = cfg.profiles.get(n);
            let key = mask_api_key(p.and_then(|p| p.api_key.as_deref()));
            let mark = if n == &cfg.active_profile {
                "  ●"
            } else {
                ""
            };
            format!("{n}  ·  {key}{mark}")
        })
        .collect();
    let add_idx = labels.len();
    labels.push("＋  Add account".into());
    labels.push("Re-authenticate (login)".into());
    labels.push("Log out".into());
    let current = if let Some(agent) = flag_agent(parsed) {
        cfg.agent_binding(&agent)
            .and_then(|b| b.profile.as_ref())
            .and_then(|n| names.iter().position(|x| x == n))
            .or_else(|| names.iter().position(|n| n == &cfg.active_profile))
    } else {
        names.iter().position(|n| n == &cfg.active_profile)
    };
    let title = if let Some(agent) = flag_agent(parsed) {
        format!("{agent} account")
    } else {
        "Account".into()
    };
    let idx = pick_list(
        &title,
        &["enter to switch    newest profiles are listed too".into()],
        &labels,
        current,
    )?;
    if idx < names.len() {
        return run_account_use(parsed, env, &names[idx]);
    }
    if idx == add_idx {
        let name = term::prompt("New account name (Enter for \"default\"): ")?;
        let name = name.trim();
        let name = if name.is_empty() { "default" } else { name };
        if !valid_account_name(name) {
            return Err(format!(
                "Invalid account name \"{name}\". Use letters, digits, \".\", \"_\", \"-\"."
            ));
        }
        let mut flags = parsed.flags.clone();
        flags.insert("profile".into(), FlagValue::Value(name.into()));
        let next = ParsedArgs {
            command: "login".into(),
            flags,
            passthrough: Vec::new(),
        };
        return run_login(&next, env);
    }
    if idx == add_idx + 1 {
        return run_login(parsed, env);
    }
    run_logout(parsed, env)
}

/// `x` on a row: clear the override so the built-in default applies again.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
fn config_reset_row(path: &std::path::Path, kind: SettingKind) -> Result<i32, String> {
    let mut cfg = load_config_if_present(path).unwrap_or_default();
    match kind {
        SettingKind::AutoUpdate => {
            if cfg.auto_update.is_none() {
                println!("{}", term::dim("auto-update already at default (enabled)"));
                return Ok(0);
            }
            cfg.auto_update = None;
            write_config(&cfg, path)?;
            println!(
                "{}  auto-update reset to default (enabled)",
                term::ok("Saved")
            );
            Ok(0)
        }
        SettingKind::Channel => {
            if cfg.channel.is_none() {
                println!("{}", term::dim("channel already at default (stable)"));
                return Ok(0);
            }
            cfg.channel = None;
            write_config(&cfg, path)?;
            println!("{}  channel reset to default (stable)", term::ok("Saved"));
            Ok(0)
        }
        SettingKind::Account
        | SettingKind::ApiKey
        | SettingKind::Install(_)
        | SettingKind::ToolCommand(_)
        | SettingKind::Mapping => Ok(0),
        SettingKind::AgentAccount(id) | SettingKind::AgentKey(id) | SettingKind::AgentModel(id) => {
            let (label, was_set) = {
                let Some(b) = cfg.agents.get_mut(id) else {
                    println!("{}", term::dim(&format!("{id} already at session default")));
                    return Ok(0);
                };
                match kind {
                    SettingKind::AgentAccount(_) => ("account", b.profile.take().is_some()),
                    SettingKind::AgentKey(_) => ("key", b.api_key.take().is_some()),
                    _ => ("model", b.default_model.take().is_some()),
                }
            };
            cfg.prune_empty_agents();
            if !was_set {
                println!(
                    "{}",
                    term::dim(&format!("{id} {label} already at session default"))
                );
                return Ok(0);
            }
            write_config(&cfg, path)?;
            println!(
                "{}  {id} {label} reset to session default",
                term::ok("Saved")
            );
            Ok(0)
        }
        SettingKind::AgentRouting(id, field) => {
            let was_set = {
                let Some(b) = cfg.agents.get_mut(id) else {
                    println!(
                        "{}",
                        term::dim(&format!("{id} {} already off", field.label()))
                    );
                    return Ok(0);
                };
                match field {
                    RoutingField::Exacto => {
                        let on = b.routing.wants_exacto();
                        b.routing.set_exacto(false);
                        on
                    }
                    RoutingField::Tools => {
                        let on = b.routing.requires_tools();
                        b.routing.set_require_tools(false);
                        on
                    }
                    RoutingField::MinContext => {
                        let on = b.routing.requires_1m_context();
                        b.routing.set_require_1m(false);
                        on
                    }
                }
            };
            cfg.prune_empty_agents();
            if !was_set {
                println!(
                    "{}",
                    term::dim(&format!("{id} {} already off", field.label()))
                );
                return Ok(0);
            }
            write_config(&cfg, path)?;
            println!("{}  {id} {} off", term::ok("Saved"), field.label());
            Ok(0)
        }
        SettingKind::GatewayDiscovery => {
            let mut cfg = load_config_if_present(path).unwrap_or_default();
            if let Some(t) = cfg.tools.get_mut("claude") {
                t.enable_gateway_model_discovery = true;
            }
            write_config(&cfg, path)?;
            println!("{}  gateway discovery reset to on", term::ok("Saved"));
            Ok(0)
        }
        SettingKind::Model(_) | SettingKind::Agent => {
            let name = cfg.active_profile.clone();
            let Some(p) = cfg.profiles.get_mut(&name) else {
                return Err(no_key_error());
            };
            let (label, was_set) = match kind {
                SettingKind::Model("haiku") => ("haiku", p.claude_haiku.take().is_some()),
                SettingKind::Model("sonnet") => ("sonnet", p.claude_sonnet.take().is_some()),
                SettingKind::Model("opus") => ("opus", p.claude_opus.take().is_some()),
                SettingKind::Model("fable") => ("fable", p.claude_fable.take().is_some()),
                SettingKind::Model(_) => ("default model", p.default_model.take().is_some()),
                _ => ("coding agent", p.default_tool.take().is_some()),
            };
            if !was_set {
                println!("{}", term::dim(&format!("{label} already at default")));
                return Ok(0);
            }
            write_config(&cfg, path)?;
            if label == "default model" {
                println!(
                    "{}  default model reset to anyrouter/auto",
                    term::ok("Saved")
                );
            } else {
                println!("{}  {} reset to default", term::ok("Saved"), label);
            }
            Ok(0)
        }
    }
}

/// Non-native builds (wasm demo) keep the flat action menu.
#[cfg(not(feature = "native"))]
fn config_menu_loop_legacy(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
) -> Result<i32, String> {
    let items = vec![
        "Switch key".into(),
        "Switch account".into(),
        "Switch model".into(),
        "Credits".into(),
        "Sign in".into(),
        "Log out".into(),
        "Done".into(),
    ];
    loop {
        let header = config_tui_header(path);
        let Some(idx) = tui_menu_select("Config", header, items.clone())? else {
            return Ok(0);
        };
        let result = match idx {
            0 => {
                let mut next = parsed.clone();
                next.command = "keys".into();
                next.passthrough = vec!["use".into()];
                run_keys(&next, env)
            }
            1 => {
                let mut next = parsed.clone();
                next.passthrough = Vec::new();
                run_auth_switch(&next, env)
            }
            2 => {
                let mut next = parsed.clone();
                next.command = "models".into();
                next.flags.insert("pick".into(), FlagValue::Bool(true));
                next.passthrough = Vec::new();
                run_models(&next, env)
            }
            3 => run_usage(parsed, env),
            4 => run_login(parsed, env),
            5 => run_logout(parsed, env),
            _ => return Ok(0),
        };
        if let Err(err) = result {
            eprintln!("{}", term::err(&err));
        }
    }
}

#[cfg(not(feature = "native"))]
fn config_tui_header(path: &std::path::Path) -> Vec<String> {
    let cfg = load_config_if_present(path).unwrap_or_default();
    let profile = cfg.profiles.get(&cfg.active_profile);
    vec![
        format!(
            "account  {}  {}",
            cfg.active_profile,
            mask_api_key(profile.and_then(|p| p.api_key.as_deref()))
        ),
        format!(
            "model    {}",
            session_model_label(profile.map(|p| p.default_model()).unwrap_or("auto"))
        ),
        format!("file     {}", path.display()),
    ]
}

fn run_config(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let sub = parsed.passthrough.first().map(String::as_str);
    match sub {
        Some("path") => {
            println!("{}", path.display());
            Ok(0)
        }
        Some("use") => {
            let name = parsed
                .passthrough
                .get(1)
                .cloned()
                .ok_or_else(|| hint("Usage: {bin} config use <account>"))?;
            run_account_use(parsed, env, &name)
        }
        Some("get") => {
            if parsed.flag_true("json") {
                let cfg = load_config_if_present(&path).unwrap_or_default();
                let profile = cfg.profiles.get(&cfg.active_profile);
                let payload = serde_json::json!({
                    "path": path.display().to_string(),
                    "active_profile": cfg.active_profile,
                    "api_key": mask_api_key(profile.and_then(|p| p.api_key.as_deref())),
                    "default_model": profile.map(|p| display_model_id(p.default_model())),
                    "claude_haiku": profile.map(|p| p.claude_haiku()),
                    "claude_sonnet": profile.map(|p| p.claude_sonnet()),
                    "claude_opus": profile.map(|p| p.claude_opus()),
                    "accounts": cfg.profiles.keys().cloned().collect::<Vec<_>>(),
                    "auto_update": cfg.auto_update(),
                    "channel": cfg.channel(),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
                );
                Ok(0)
            } else {
                print_config_status(parsed, env, &path)?;
                Ok(0)
            }
        }
        None if parsed.flag_true("json") => {
            let mut next = parsed.clone();
            next.passthrough = vec!["get".into()];
            run_config(&next, env)
        }
        None if tui_wants_dump(parsed, env) || term::is_interactive() => {
            run_config_tui(parsed, env)
        }
        None => {
            print_config_status(parsed, env, &path)?;
            println!(
                "{}",
                term::dim(&hint("Run `{bin} config` in a terminal to pick key, model, and account."))
            );
            Ok(0)
        }
        Some(other) => Err(hint(&format!(
            "Unknown config command \"{other}\". Try: {{bin}} config · {{bin}} config path · {{bin}} config use <account>"
        ))),
    }
}

fn catalog_lookup_enabled(env: &BTreeMap<String, String>) -> bool {
    match env.get("ANYR_NO_CATALOG").map(|s| s.as_str()) {
        Some("1" | "true" | "TRUE" | "yes") => false,
        _ => true,
    }
}

struct ResolvedModel {
    id: String,
    context_window: Option<i64>,
}

/// Auto (unset / `auto` / `anyrouter/auto`) resolves to the catalog's most-used
/// model this week. A user-pinned id is kept. Failures keep the requested id.
fn resolve_session_model(
    requested: &str,
    base: &str,
    key: Option<&str>,
    env: &BTreeMap<String, String>,
) -> ResolvedModel {
    let requested = catalog_model_id(requested);
    if !catalog_lookup_enabled(env) {
        return ResolvedModel {
            id: requested,
            context_window: None,
        };
    }
    let Ok(models) = fetch_models(base, key) else {
        return ResolvedModel {
            id: requested,
            context_window: None,
        };
    };
    let id = if is_auto_model(&requested) {
        most_used_model_id(&models).unwrap_or(requested)
    } else {
        requested
    };
    let context_window = models
        .iter()
        .find(|m| catalog_model_id(&m.id) == id)
        .and_then(|m| m.context_length);
    ResolvedModel { id, context_window }
}

fn run_launch(
    tool_name: &str,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let stored = existing
        .as_ref()
        .and_then(|c| profile_for_agent(c, &parsed.flags, env, tool_name));
    let key = if let Some(key) =
        resolve_launch_api_key(&parsed.flags, env, existing.as_ref(), tool_name)
    {
        key
    } else {
        let base = resolve_base_url(&parsed.flags, stored);
        let acquired = acquire_api_key(&parsed.flags, env, &base, Some(tool_name))?;
        persist_login(parsed, env, &acquired.api_key, &acquired.source)?;
        acquired.api_key
    };
    let existing = load_config_if_present(&path);
    let stored = existing
        .as_ref()
        .and_then(|c| profile_for_agent(c, &parsed.flags, env, tool_name));
    let base = resolve_base_url(&parsed.flags, stored);
    let mut profile = stored
        .cloned()
        .unwrap_or_else(|| default_profile_for_env(Some(&base), Some(&key)));
    profile.base_url = Some(base.clone());
    let aliases_changed = apply_claude_alias_flags(&mut profile, parsed);
    let tool = resolve_tool(existing.as_ref(), tool_name)?;
    let requested = catalog_model_id(&resolve_launch_model(
        &parsed.flags,
        existing.as_ref(),
        &profile,
        tool_name,
    ));
    let resolved = resolve_session_model(&requested, &base, Some(&key), env);
    let model = resolved.id;
    let effort = normalize_effort(get_string_flag(&parsed.flags, "effort").as_deref())?;
    let model_mode = if is_auto_model(&model) {
        "auto"
    } else {
        "concrete"
    };
    let mut env_map = build_tool_env(BuildToolEnvInput {
        tool_name,
        tool: &tool,
        profile: &profile,
        api_key: &key,
        model: &model,
        effort: effort.as_deref(),
        context_window: resolved.context_window,
        model_map: None,
    });
    let routing = existing
        .as_ref()
        .and_then(|c| c.agent_binding(tool_name))
        .map(|b| b.routing.clone())
        .unwrap_or_default();
    apply_routing_env(&mut env_map, &routing, tool_name);
    if tool_name == "pi" {
        prepare_pi_wrapper(&mut env_map, &path, &profile, &tool, &model)?;
    }
    let mut args = Vec::new();
    args.extend(effort_args_for(tool_name, effort.as_deref()));
    args.extend(provider_args_for(tool_name, &profile));
    args.extend(model_args_for(tool_name, &model, model_mode));
    // --yolo is shorthand for Claude Code's full-permission flag; other tools
    // don't have an equivalent, so it only maps there.
    if tool_name == "claude" && parsed.flag_true("yolo") {
        args.push("--dangerously-skip-permissions".into());
    }
    args.extend(parsed.passthrough.clone());
    let command = get_string_flag(&parsed.flags, "command-path")
        .or_else(|| env_command_path(tool_name, env))
        .unwrap_or_else(|| tool.command.clone());
    if parsed.flag_true("dry-run") {
        println!("{}", render_dry_run(&command, &args, &env_map));
        return Ok(0);
    }
    let resolved = ensure_tool_installed(tool_name, &command, parsed.flag_true("install"))?;
    // Persist on top of the existing config when there is one; a fresh setup
    // (no config yet) gets one so last_tool and the model are remembered.
    let mut cfg = existing.clone().unwrap_or_else(|| crate::config::Config {
        active_profile: DEFAULT_PROFILE.into(),
        ..crate::config::Config::default()
    });
    // First launch on this machine: no stored profile yet — seed one from
    // what this launch resolved so the key, base URL, and model stick.
    cfg.profiles
        .entry(cfg.active_profile.clone())
        .or_insert_with(|| profile.clone());
    cfg.last_tool = Some(tool_name.to_string());
    if aliases_changed {
        if let Some(p) = cfg.profiles.get_mut(&cfg.active_profile) {
            p.claude_haiku = profile.claude_haiku.clone();
            p.claude_sonnet = profile.claude_sonnet.clone();
            p.claude_opus = profile.claude_opus.clone();
            p.claude_fable = profile.claude_fable.clone();
        }
    }
    // Remember the model this launch used as the session default, so a bare
    // `{bin} claude` next time starts with it. Also pin it on this agent so a
    // later grok launch does not inherit claude's model.
    if let Some(flag_model) = get_string_flag(&parsed.flags, "model") {
        let id = catalog_model_id(&flag_model);
        let stored = if is_auto_model(&id) { None } else { Some(id) };
        if let Some(p) = cfg.profiles.get_mut(&cfg.active_profile) {
            // Auto stays unset so the next launch re-picks the most-used model.
            p.default_model = stored.clone();
        }
        cfg.agent_binding_mut(tool_name).default_model = stored;
        cfg.prune_empty_agents();
    }
    let _ = write_config(&cfg, &path);
    let _updater = crate::upgrade::start_session_checker(env);
    Ok(spawn_child(&resolved, &args, &env_map))
}

fn run_logout(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let Some(mut cfg) = load_config_if_present(&path) else {
        return Err(no_key_error());
    };
    let name =
        get_string_flag(&parsed.flags, "profile").unwrap_or_else(|| cfg.active_profile.clone());
    let Some(profile) = cfg.profiles.get_mut(&name) else {
        return Err(format!("Account \"{name}\" was not found."));
    };
    profile.api_key = None;
    profile.management_key = None;
    write_config(&cfg, &path)?;
    println!(
        "{}  cleared keys for {}",
        term::ok("Logged out"),
        term::accent(&name)
    );
    Ok(0)
}

fn run_account_use(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    name: &str,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    if let Some(agent) = flag_agent(parsed) {
        return save_agent_account(&path, &agent, name);
    }
    let cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
    let cfg = set_active_profile(cfg, name)?;
    write_config(&cfg, &path)?;
    println!(
        "{}  active account  {}",
        term::ok("Switched"),
        term::accent(name)
    );
    Ok(0)
}

fn run_account(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let sub = parsed
        .passthrough
        .first()
        .map(String::as_str)
        .unwrap_or("list");
    match sub {
        "list" => {
            let Some(cfg) = load_config_if_present(&path) else {
                return Err(no_key_error());
            };
            if parsed.flag_true("json") {
                let rows: Vec<_> = cfg
                    .profiles
                    .iter()
                    .map(|(name, p)| {
                        serde_json::json!({
                            "name": name,
                            "active": name == &cfg.active_profile,
                            "default_model": p.default_model(),
                            "has_key": p.api_key.as_ref().is_some_and(|s| !s.is_empty()),
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".into())
                );
                return Ok(0);
            }
            for (name, profile) in &cfg.profiles {
                let marker = if name == &cfg.active_profile {
                    "*"
                } else {
                    " "
                };
                println!(
                    "{marker} {}  {}  {}",
                    term::accent(name),
                    term::model_id(&session_model_label(profile.default_model())),
                    mask_api_key(profile.api_key.as_deref())
                );
            }
            if cfg.profiles.is_empty() {
                println!("{}", term::dim(&hint("No accounts. Run: {bin} login")));
            }
            Ok(0)
        }
        "use" => {
            let name = parsed
                .passthrough
                .get(1)
                .cloned()
                .ok_or_else(|| hint("Usage: {bin} account use <name>"))?;
            run_account_use(parsed, env, &name)
        }
        "add" => {
            let name = parsed
                .passthrough
                .get(1)
                .cloned()
                .unwrap_or_else(|| DEFAULT_PROFILE.into());
            if !valid_account_name(&name) {
                return Err(format!(
                    "Invalid account name \"{name}\". Use letters, digits, \".\", \"_\", \"-\"."
                ));
            }
            let mut flags = parsed.flags.clone();
            flags.insert("profile".into(), FlagValue::Value(name));
            let next = ParsedArgs {
                command: "login".into(),
                flags,
                passthrough: Vec::new(),
            };
            run_login(&next, env)
        }
        "rename" => {
            let old = parsed
                .passthrough
                .get(1)
                .cloned()
                .ok_or_else(|| hint("Usage: {bin} account rename <old> <new>"))?;
            let new = parsed
                .passthrough
                .get(2)
                .cloned()
                .ok_or_else(|| hint("Usage: {bin} account rename <old> <new>"))?;
            if !valid_account_name(&new) {
                return Err(format!(
                    "Invalid account name \"{new}\". Use letters, digits, \".\", \"_\", \"-\"."
                ));
            }
            let mut cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
            let profile = cfg
                .profiles
                .remove(&old)
                .ok_or_else(|| format!("Account \"{old}\" was not found."))?;
            if cfg.profiles.contains_key(&new) {
                return Err(format!("Account \"{new}\" already exists."));
            }
            cfg.profiles.insert(new.clone(), profile);
            if cfg.active_profile == old {
                cfg.active_profile = new.clone();
            }
            write_config(&cfg, &path)?;
            println!("{}  {old} → {}", term::ok("Renamed"), term::accent(&new));
            Ok(0)
        }
        "remove" => {
            let name = parsed
                .passthrough
                .get(1)
                .cloned()
                .ok_or_else(|| hint("Usage: {bin} account remove <name>"))?;
            let mut cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
            if cfg.active_profile == name {
                return Err(hint(&format!(
                    "\"{name}\" is the active account. Switch first: {{bin}} account use <other>"
                )));
            }
            if cfg.profiles.remove(&name).is_none() {
                return Err(format!("Account \"{name}\" was not found."));
            }
            write_config(&cfg, &path)?;
            println!("{}  removed {}", term::ok("Removed"), term::accent(&name));
            Ok(0)
        }
        other => Err(format!(
            "Unknown account subcommand \"{other}\". Try: list | use | add | rename | remove"
        )),
    }
}

fn keys_credential(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<(std::path::PathBuf, crate::config::Config, String, String), String> {
    let path = config_path(parsed, env);
    let cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
    let name =
        get_string_flag(&parsed.flags, "profile").unwrap_or_else(|| cfg.active_profile.clone());
    let profile = cfg
        .profiles
        .get(&name)
        .ok_or_else(|| format!("Profile \"{name}\" was not found in AnyRouter config."))?;
    let base = resolve_base_url(&parsed.flags, Some(profile));
    let api_key = resolve_api_key(&parsed.flags, env, Some(profile))
        .ok_or_else(|| hint("No stored credential. Run \"{bin} login\" first."))?;
    Ok((path, cfg, base, api_key))
}

fn default_key_name() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "device".into());
    let short = host.split('.').next().unwrap_or("device");
    format!("cli-{short}").chars().take(40).collect()
}

fn run_keys(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let sub = parsed
        .passthrough
        .first()
        .map(String::as_str)
        .unwrap_or("list");
    match sub {
        "list" => {
            let (_path, _cfg, base, api_key) = keys_credential(parsed, env)?;
            let rows = crate::http::keys_newest_first(fetch_keys(&base, &api_key)?);
            if parsed.flag_true("json") {
                let payload: Vec<_> = rows
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "name": r.name,
                            "hash": r.hash,
                            "masked": r.masked,
                            "created_at": r.created_at,
                            "last_used_at": r.last_used_at,
                            "active": r.active,
                            "current": is_active_key_row(&r.masked, Some(&api_key)),
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "[]".into())
                );
                return Ok(0);
            }
            if rows.is_empty() {
                println!(
                    "{}",
                    term::dim(&hint("No API keys. Create one: {bin} keys create"))
                );
                return Ok(0);
            }
            let name_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(4).max(4);
            for r in &rows {
                let marker = if is_active_key_row(&r.masked, Some(&api_key)) {
                    "*"
                } else {
                    " "
                };
                let state = if r.active { "" } else { "  (disabled)" };
                println!(
                    "{marker} {:name_w$}  {}  {}{state}",
                    r.name,
                    r.masked,
                    r.last_used_at.as_deref().unwrap_or("never used")
                );
            }
            println!();
            println!(
                "{}",
                term::dim(&hint("* = key this profile uses · switch: {bin} keys use"))
            );
            Ok(0)
        }
        "create" => {
            let (path, mut cfg, base, cred) = keys_credential(parsed, env)?;
            let name = parsed
                .passthrough
                .get(1)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(default_key_name);
            let key = create_key(&base, &cred, &name)?;
            println!("Created \"{name}\":\n\n  {key}\n\nShown once — store it now.");
            let save = parsed.flag_true("yes")
                || (term::is_interactive()
                    && term::confirm("Use this key for the current profile?"));
            if save {
                let active = cfg.active_profile.clone();
                if let Some(p) = cfg.profiles.get_mut(&active) {
                    p.api_key = Some(key);
                }
                write_config(&cfg, &path)?;
                println!("{}  saved to config.yaml", term::ok("Saved"));
            }
            Ok(0)
        }
        "use" => {
            let (path, mut cfg, base, api_key) = keys_credential(parsed, env)?;
            let rows = crate::http::keys_newest_first(
                fetch_keys(&base, &api_key)?
                    .into_iter()
                    .filter(|r| r.active)
                    .collect(),
            );
            if rows.is_empty() {
                return Err(hint("No active keys. Create one: {bin} keys create"));
            }
            let hash_arg = parsed.passthrough.get(1).cloned();
            let row = if let Some(hash) = hash_arg {
                let matches: Vec<_> = rows
                    .iter()
                    .filter(|r| r.hash == hash || r.hash.starts_with(&hash))
                    .collect();
                match matches.as_slice() {
                    [one] => (*one).clone(),
                    [] => {
                        return Err(hint(&format!(
                            "No key matches \"{hash}\". See: {{bin}} keys list"
                        )))
                    }
                    _ => {
                        return Err(format!(
                            "\"{hash}\" matches {} keys — use a longer hash prefix.",
                            matches.len()
                        ))
                    }
                }
            } else if term::is_interactive() {
                let current = rows
                    .iter()
                    .position(|r| is_active_key_row(&r.masked, Some(&api_key)))
                    .or(Some(0));
                let labels: Vec<String> = rows
                    .iter()
                    .enumerate()
                    .map(|(i, r)| key_pick_label(r, current == Some(i)))
                    .collect();
                let idx = pick_list(
                    "API key",
                    &[
                        "newest first · type to search".into(),
                        format!("current  {}", mask_api_key(Some(&api_key))),
                    ],
                    &labels,
                    current,
                )?;
                rows[idx].clone()
            } else {
                return Err(hint(
                    "Usage: {bin} keys use <hash>   (interactive picker needs a terminal)",
                ));
            };
            let revealed = reveal_key(&base, &api_key, &row.hash)?;
            if let Some(agent) = flag_agent(parsed) {
                return save_agent_key(&path, &agent, &revealed);
            }
            let active = cfg.active_profile.clone();
            if let Some(p) = cfg.profiles.get_mut(&active) {
                p.api_key = Some(revealed);
            }
            write_config(&cfg, &path)?;
            println!(
                "{}  now using {} ({})",
                term::ok("Switched"),
                term::accent(&row.name),
                mask_api_key(cfg.profiles.get(&active).and_then(|p| p.api_key.as_deref()))
            );
            Ok(0)
        }
        "revoke" => {
            let hash = parsed.passthrough.get(1).cloned().ok_or_else(|| {
                hint("Usage: {bin} keys revoke <hash>   (find hashes: {bin} keys list)")
            })?;
            let (_path, _cfg, base, cred) = keys_credential(parsed, env)?;
            let rows = fetch_keys(&base, &cred)?;
            let matches: Vec<_> = rows
                .iter()
                .filter(|r| r.hash == hash || r.hash.starts_with(&hash))
                .collect();
            let row = match matches.as_slice() {
                [one] => *one,
                [] => {
                    return Err(hint(&format!(
                        "No key matches \"{hash}\". See: {{bin}} keys list"
                    )))
                }
                _ => {
                    return Err(format!(
                        "\"{hash}\" matches {} keys — use a longer hash prefix.",
                        matches.len()
                    ))
                }
            };
            if !parsed.flag_true("yes") {
                if !term::is_interactive() {
                    return Err(
                        "Revoking a key is destructive; pass --yes to run non-interactively."
                            .into(),
                    );
                }
                if !term::confirm(&format!("Revoke \"{}\" ({})?", row.name, row.masked)) {
                    return Ok(1);
                }
            }
            delete_key(&base, &cred, &row.hash)?;
            println!(
                "{}  revoked \"{}\" ({})",
                term::ok("Revoked"),
                row.name,
                row.masked
            );
            Ok(0)
        }
        other => Err(format!(
            "Unknown keys subcommand \"{other}\". Try: list | create | use | revoke"
        )),
    }
}

fn resolve_latest_key(base: &str, current: &str) -> String {
    let Ok(rows) = fetch_keys(base, current) else {
        return current.to_string();
    };
    let rows = crate::http::keys_newest_first(rows.into_iter().filter(|r| r.active).collect());
    let Some(latest) = rows.first() else {
        return current.to_string();
    };
    if is_active_key_row(&latest.masked, Some(current)) {
        return current.to_string();
    }
    if !latest.can_reveal {
        // Reveal would 409 for pre-reveal-support rows; the stored key stays.
        return current.to_string();
    }
    reveal_key(base, current, &latest.hash).unwrap_or_else(|_| current.to_string())
}

fn key_pick_label(row: &crate::http::RemoteKey, current: bool) -> String {
    let mut parts = vec![row.name.clone(), row.masked.clone()];
    if let Some(created) = row.created_at.as_deref() {
        parts.push(created.get(..10).unwrap_or(created).to_string());
    }
    if current {
        parts.push("●".into());
    }
    parts.join("  ·  ")
}

fn stored_api_key(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
) -> Option<String> {
    let existing = load_config_if_present(path);
    let profile = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    resolve_api_key(&parsed.flags, env, profile)
}

fn agent_binding_detail(cfg: &crate::config::Config, id: &str, signed_in: bool) -> String {
    let profile = cfg.profiles.get(&cfg.active_profile);
    let binding = cfg.agent_binding(id);
    let model = binding
        .and_then(|b| b.default_model.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(session_model_label)
        .unwrap_or_else(|| {
            session_model_label(profile.map(|p| p.default_model()).unwrap_or("auto"))
        });
    let account = binding
        .and_then(|b| b.profile.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(cfg.active_profile.as_str());
    let key = if signed_in {
        if let Some(k) = binding
            .and_then(|b| b.api_key.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            mask_api_key(Some(k))
        } else {
            mask_api_key(profile.and_then(|p| p.api_key.as_deref()))
        }
    } else {
        "(not signed in)".into()
    };
    let mut parts = vec![model, account.to_string(), key];
    if let Some(r) = binding.map(|b| &b.routing) {
        if r.wants_exacto() {
            parts.push("exacto".into());
        }
        if r.requires_tools() {
            parts.push("tools".into());
        }
        if r.requires_1m_context() {
            parts.push("1M".into());
        }
    }
    parts.join("  ·  ")
}

/// Palette entries: launch rows first (bindings visible per agent), then
/// model/account/key for the highlighted agent on the same screen, then more.
/// `signed_in` gates the launch group exactly like the old dialog did.
#[cfg(feature = "native")]
fn launcher_palette(
    path: &PathBuf,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    credits: &mut CreditsCache,
) -> (Vec<String>, Vec<crate::tui::PaletteEntry>) {
    let cfg = load_config_if_present(path).unwrap_or_default();
    let profile = cfg.profiles.get(&cfg.active_profile);
    let signed_in = resolve_api_key(&parsed.flags, env, profile).is_some();
    let command_for = |id: &str| {
        crate::spawn::resolve_tool(Some(&cfg), id)
            .map(|t| t.command)
            .unwrap_or_else(|_| id.to_string())
    };
    let present = available_agents(env, command_for);
    let last = cfg
        .last_tool
        .clone()
        .or_else(|| profile.and_then(|p| p.default_tool.clone()))
        .or_else(|| get_string_flag(&parsed.flags, "tool"))
        .or_else(|| present.first().map(|(id, _)| (*id).to_string()))
        .unwrap_or_else(|| "claude".into());

    // Compact status: bindings live on each agent row, not a 5-line header.
    let dump_or_pipe = tui_wants_dump(parsed, env) || !term::is_interactive();
    let credits_line = format!("credits  {}", credits.peek_credits());
    let account_line = if dump_or_pipe {
        format!("account  {}", cfg.active_profile)
    } else if let Some(label) = credits.peek_identity().map(|me| me.display_label()) {
        format!("account  {label}")
    } else {
        format!("account  {}", cfg.active_profile)
    };
    let header = vec![account_line, credits_line];

    #[cfg(feature = "native")]
    use crate::tui::PaletteEntry;
    let mut entries = Vec::new();
    if signed_in {
        push_launch_entries(&mut entries, &present, &last, |id| {
            agent_binding_detail(&cfg, id, signed_in)
        });
    } else {
        entries.push(PaletteEntry::new(
            "login",
            "sign in / add key",
            "account",
            "Login / sign in",
        ));
    }
    let hub = if present.iter().any(|(id, _)| *id == last.as_str()) {
        last.clone()
    } else {
        present
            .first()
            .map(|(id, _)| (*id).to_string())
            .unwrap_or(last.clone())
    };
    push_agent_configure_entries(&mut entries, &hub, &cfg);
    entries.push(PaletteEntry::new(
        "install…",
        "install a coding agent",
        "more",
        "Install agent",
    ));
    entries.push(PaletteEntry::new(
        "config…",
        "accounts · keys · agent",
        "more",
        "Config",
    ));
    entries.push(PaletteEntry::new("quit", "esc works too", "more", "Quit"));
    (header, entries)
}

/// Non-native twin of `launcher_palette` — same rows, plain inline entries.
#[cfg(not(feature = "native"))]
fn launcher_palette(
    path: &PathBuf,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    _credits: &mut CreditsCache,
) -> (Vec<String>, Vec<InlineEntry>) {
    let cfg = load_config_if_present(path).unwrap_or_default();
    let profile = cfg.profiles.get(&cfg.active_profile);
    let signed_in = launcher_signed_in(path, parsed, env);
    let last = launcher_last_tool(path, parsed, env);
    let header = vec![
        format!("account  {}", cfg.active_profile),
        "credits  -".to_string(),
    ];
    let mut entries = Vec::new();
    let present = available_agents(env, |id| tool_command_for(path, id));
    if signed_in {
        if present.is_empty() {
            entries.push(InlineEntry::new(
                "install an agent…",
                "none detected on PATH",
                "launch",
                "Install agent",
            ));
        } else {
            let last = if present.iter().any(|(id, _)| *id == last) {
                last.clone()
            } else {
                present[0].0.to_string()
            };
            let detail = agent_binding_detail(&cfg, &last, signed_in);
            entries.push(InlineEntry::new(
                last.clone(),
                detail,
                "launch",
                format!("Launch {last}"),
            ));
            for (id, _) in present
                .iter()
                .copied()
                .filter(|(id, _)| *id != last.as_str())
            {
                entries.push(InlineEntry::new(
                    id,
                    agent_binding_detail(&cfg, id, signed_in),
                    "launch",
                    format!("Launch {id}"),
                ));
            }
            push_inline_configure(&mut entries, &last, &cfg);
        }
    } else {
        entries.push(InlineEntry::new(
            "login",
            "sign in / add key",
            "account",
            "Login / sign in",
        ));
        push_inline_configure(&mut entries, &last, &cfg);
    }
    entries.push(InlineEntry::new(
        "install…",
        "install a coding agent",
        "more",
        "Install agent",
    ));
    entries.push(InlineEntry::new(
        "config…",
        "accounts · keys · agent",
        "more",
        "Config",
    ));
    entries.push(InlineEntry::new("quit", "esc works too", "more", "Quit"));
    (header, entries)
}

#[cfg(not(feature = "native"))]
fn push_inline_configure(entries: &mut Vec<InlineEntry>, agent: &str, cfg: &crate::config::Config) {
    let routing = cfg
        .agent_binding(agent)
        .map(|b| b.routing.clone())
        .unwrap_or_default();
    entries.push(InlineEntry::new(
        "model…",
        palette_bind_detail(agent),
        format!("configure · {agent}"),
        format!("Switch model {agent}"),
    ));
    entries.push(InlineEntry::new(
        "account…",
        palette_bind_detail(agent),
        format!("configure · {agent}"),
        format!("Switch account {agent}"),
    ));
    entries.push(InlineEntry::new(
        "key…",
        palette_bind_detail(agent),
        format!("configure · {agent}"),
        format!("Switch key {agent}"),
    ));
    entries.push(InlineEntry::new(
        "exacto",
        routing_toggle_detail(routing.wants_exacto(), agent),
        format!("configure · {agent}"),
        format!("Toggle exacto {agent}"),
    ));
    entries.push(InlineEntry::new(
        "tools",
        routing_toggle_detail(routing.requires_tools(), agent),
        format!("configure · {agent}"),
        format!("Toggle tools {agent}"),
    ));
    entries.push(InlineEntry::new(
        "1M ctx",
        routing_toggle_detail(routing.requires_1m_context(), agent),
        format!("configure · {agent}"),
        format!("Toggle 1m {agent}"),
    ));
}

fn run_menu(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let dumping = tui_wants_dump(parsed, env);

    if dumping {
        let (header, entries) = launcher_palette(&path, parsed, env, &mut CreditsCache::fresh());
        print!("{}", tui_dump_palette(entries, header, env));
        return Ok(0);
    }

    if !term::is_interactive() {
        let (_, entries) = launcher_palette(&path, parsed, env, &mut CreditsCache::fresh());
        println!(
            "{}",
            entries
                .iter()
                .map(|e| e.label.clone())
                .collect::<Vec<_>>()
                .join("\n")
        );
        return Ok(0);
    }

    // Loop until Quit or a coding-agent launch takes over the process.
    // Compact HUD by default. ANYR_TUI=1 restores the palette.
    let cache = Arc::new(Mutex::new(CreditsCache::fresh()));
    #[cfg(feature = "native")]
    if term::is_interactive() {
        let cfg = load_config_if_present(&path).unwrap_or_default();
        let profile = cfg.profiles.get(&cfg.active_profile);
        let base = resolve_base_url(&parsed.flags, profile);
        let key = resolve_api_key(&parsed.flags, env, profile);
        kick_credits_refresh(&cache, base, key);
    }
    let inline = !launcher_uses_palette();
    loop {
        let (header, entries) = {
            let mut credits = cache.lock().unwrap_or_else(|p| p.into_inner());
            launcher_palette(&path, parsed, env, &mut credits)
        };
        let idx = if inline {
            let labels: Vec<String> = entries
                .iter()
                .map(|e| {
                    if e.detail.is_empty() {
                        e.label.clone()
                    } else {
                        format!("{}  —  {}", e.label, e.detail)
                    }
                })
                .collect();
            for h in &header {
                eprintln!("{}", term::dim(h));
            }
            match term::pick_numbered("anyr", &labels, Some(0)) {
                Ok(i) => Some(i),
                Err(err) if err == "Cancelled." => None,
                Err(err) => return Err(err),
            }
        } else {
            pick_palette_action(header, entries.clone(), &cache)?
        };
        let Some(idx) = idx else {
            return Ok(0);
        };
        let action = entries[idx].action.clone();
        match launcher_dispatch(&action, parsed, env, &path)? {
            LauncherNext::Continue => {}
            LauncherNext::Exit(code) => return Ok(code),
        }
    }
}

#[derive(Debug)]
enum LauncherNext {
    Continue,
    Exit(i32),
}

#[cfg(feature = "native")]
fn push_launch_entries(
    entries: &mut Vec<crate::tui::PaletteEntry>,
    present: &[(&'static str, &'static str)],
    last: &str,
    detail_for: impl Fn(&str) -> String,
) {
    use crate::tui::PaletteEntry;
    if present.is_empty() {
        entries.push(PaletteEntry::new(
            "install an agent…",
            "none detected on PATH",
            "launch",
            "Install agent",
        ));
        return;
    }
    let last = if present.iter().any(|(id, _)| *id == last) {
        last.to_string()
    } else {
        present[0].0.to_string()
    };
    entries.push(PaletteEntry::new(
        last.clone(),
        detail_for(&last),
        "launch",
        format!("Launch {last}"),
    ));
    for (id, _) in present
        .iter()
        .copied()
        .filter(|(id, _)| *id != last.as_str())
    {
        entries.push(PaletteEntry::new(
            id,
            detail_for(id),
            "launch",
            format!("Launch {id}"),
        ));
    }
}

#[cfg(feature = "native")]
fn push_agent_configure_entries(
    entries: &mut Vec<crate::tui::PaletteEntry>,
    agent: &str,
    cfg: &crate::config::Config,
) {
    use crate::tui::PaletteEntry;
    let group = format!("configure · {agent}");
    let routing = cfg
        .agent_binding(agent)
        .map(|b| b.routing.clone())
        .unwrap_or_default();
    entries.push(PaletteEntry::new(
        "model…",
        palette_bind_detail(agent),
        group.clone(),
        format!("Switch model {agent}"),
    ));
    entries.push(PaletteEntry::new(
        "account…",
        palette_bind_detail(agent),
        group.clone(),
        format!("Switch account {agent}"),
    ));
    entries.push(PaletteEntry::new(
        "key…",
        palette_bind_detail(agent),
        group.clone(),
        format!("Switch key {agent}"),
    ));
    entries.push(PaletteEntry::new(
        "exacto",
        routing_toggle_detail(routing.wants_exacto(), agent),
        group.clone(),
        format!("Toggle exacto {agent}"),
    ));
    entries.push(PaletteEntry::new(
        "tools",
        routing_toggle_detail(routing.requires_tools(), agent),
        group.clone(),
        format!("Toggle tools {agent}"),
    ));
    entries.push(PaletteEntry::new(
        "1M ctx",
        routing_toggle_detail(routing.requires_1m_context(), agent),
        group,
        format!("Toggle 1m {agent}"),
    ));
}

fn install_agent_dialog(path: &PathBuf, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let missing = missing_agents(env, |id| tool_command_for(path, id));
    if missing.is_empty() {
        println!(
            "{}",
            term::dim("Every known coding agent is already on PATH.")
        );
        return Ok(0);
    }
    let labels: Vec<String> = missing
        .iter()
        .map(|(id, label)| {
            let cmd = crate::install::tool_hint(id)
                .map(|h| h.install.to_string())
                .unwrap_or_default();
            format!("{id}  —  {label}    {cmd}")
        })
        .collect();
    let idx = pick_list(
        "Install coding agent",
        &["none of these are on PATH · enter to install".into()],
        &labels,
        Some(0),
    )?;
    let (id, _) = missing[idx];
    let command = tool_command_for(path, id);
    let resolved = ensure_tool_installed(id, &command, true)?;
    persist_tool_command(path, id, &resolved)?;
    println!("{}  {id}  {}", term::ok("Installed"), term::dim(&resolved));
    Ok(0)
}

fn persist_tool_command(path: &PathBuf, id: &str, command: &str) -> Result<(), String> {
    let mut cfg = load_config_if_present(path).unwrap_or_default();
    let mut tool = resolve_tool(Some(&cfg), id)?;
    tool.command = command.to_string();
    cfg.tools.insert(id.to_string(), tool);
    write_config(&cfg, path)
}

fn launcher_last_tool(
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

fn launcher_signed_in(path: &PathBuf, parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> bool {
    stored_api_key(parsed, env, path).is_some()
}

/// Account-info cache for the launcher / settings loops: one fetch per TTL
/// instead of one per frame render. `None` = never fetched / failed.
struct CreditsCache {
    value: Option<Result<String, ()>>,
    me: Option<Result<crate::http::MeInfo, ()>>,
    fetched_at: Option<std::time::Instant>,
    gen: u64,
}

const CREDITS_TTL: std::time::Duration = std::time::Duration::from_secs(300);

impl CreditsCache {
    fn fresh() -> Self {
        Self {
            value: None,
            me: None,
            fetched_at: None,
            gen: 0,
        }
    }

    fn peek_credits(&self) -> String {
        match &self.value {
            Some(Ok(s)) => s.clone(),
            Some(Err(())) => "(unknown)".into(),
            None => "-".into(),
        }
    }

    fn peek_identity(&self) -> Option<crate::http::MeInfo> {
        match &self.me {
            Some(Ok(me)) => Some(me.clone()),
            _ => None,
        }
    }

    /// Refresh both credits and identity when stale.
    fn refresh(&mut self, base_url: &str, api_key: Option<&str>) {
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
    fn get(&mut self, base_url: &str, api_key: Option<&str>) -> String {
        self.refresh(base_url, api_key);
        self.peek_credits()
    }

    /// Cached identity, refreshing when stale. `None` when unknown.
    fn identity(&mut self, base_url: &str, api_key: Option<&str>) -> Option<crate::http::MeInfo> {
        self.refresh(base_url, api_key);
        self.peek_identity()
    }
}

#[cfg(feature = "native")]
fn kick_credits_refresh(cache: &Arc<Mutex<CreditsCache>>, base: String, key: Option<String>) {
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
fn patch_palette_header(state: &mut crate::tui::PaletteState, credits: &CreditsCache) {
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

fn launcher_dispatch(
    action: &str,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
) -> Result<LauncherNext, String> {
    if action == "Quit" || action.starts_with("Quit") {
        return Ok(LauncherNext::Exit(0));
    }
    if action.starts_with("Launch ") && action != "Launch coding agent…" {
        let tool = action.trim_start_matches("Launch ").trim();
        if !launcher_signed_in(path, parsed, env) {
            eprintln!("{}", term::err("Sign in first (Login / sign in)."));
            return Ok(LauncherNext::Continue);
        }
        return Ok(LauncherNext::Exit(run_launch(tool, parsed, env)?));
    }
    if action == "Launch coding agent…" {
        return launch_agent_picker(parsed, env, path);
    }
    if action == "Config" {
        let code = run_config_tui(parsed, env)?;
        if code != 0 {
            return Ok(LauncherNext::Exit(code));
        }
        return Ok(LauncherNext::Continue);
    }
    if action == "Install agent" {
        match install_agent_dialog(path, env) {
            Ok(_) => {}
            Err(err) if err == "Cancelled." => {}
            Err(err) => eprintln!("{}", term::err(&err)),
        }
        return Ok(LauncherNext::Continue);
    }
    if action == "Switch model" {
        if !launcher_signed_in(path, parsed, env) {
            eprintln!("{}", term::err("Sign in first (Login / sign in)."));
            return Ok(LauncherNext::Continue);
        }
        let agent = launcher_last_tool(path, parsed, env);
        let mut next = parsed.clone();
        next.command = "models".into();
        next.flags.insert("pick".into(), FlagValue::Bool(true));
        next.flags.insert("agent".into(), FlagValue::Value(agent));
        if let Err(err) = run_models(&next, env) {
            eprintln!("{}", term::err(&err));
        }
        return Ok(LauncherNext::Continue);
    }
    if let Some(agent) = action.strip_prefix("Switch model ") {
        return switch_agent_model(parsed, env, path, agent.trim());
    }
    if action == "Switch agent" {
        match config_edit_row(parsed, env, path, SettingKind::Agent) {
            Ok(_) => {}
            Err(err) if err == "Cancelled." => {}
            Err(err) => eprintln!("{}", term::err(&err)),
        }
        return Ok(LauncherNext::Continue);
    }
    if action == "Switch account" || action == "Switch account / key" {
        let agent = launcher_last_tool(path, parsed, env);
        let mut next = parsed.clone();
        next.flags.insert("agent".into(), FlagValue::Value(agent));
        match config_account_actions(&next, env) {
            Ok(_) => {}
            Err(err) if err == "Cancelled." => {}
            Err(err) => eprintln!("{}", term::err(&err)),
        }
        return Ok(LauncherNext::Continue);
    }
    if let Some(agent) = action.strip_prefix("Switch account ") {
        return switch_agent_account(parsed, env, path, agent.trim());
    }
    if action == "Switch key" {
        if !launcher_signed_in(path, parsed, env) {
            eprintln!("{}", term::err("Sign in first (Login / sign in)."));
            return Ok(LauncherNext::Continue);
        }
        let agent = launcher_last_tool(path, parsed, env);
        let mut next = parsed.clone();
        next.command = "keys".into();
        next.passthrough = vec!["use".into()];
        next.flags.insert("agent".into(), FlagValue::Value(agent));
        match run_keys(&next, env) {
            Ok(_) => {}
            Err(err) if err == "Cancelled." => {}
            Err(err) => eprintln!("{}", term::err(&err)),
        }
        return Ok(LauncherNext::Continue);
    }
    if let Some(agent) = action.strip_prefix("Switch key ") {
        return switch_agent_key(parsed, env, path, agent.trim());
    }
    if let Some(agent) = action.strip_prefix("Toggle exacto ") {
        toggle_agent_routing_field(path, agent.trim(), RoutingField::Exacto)?;
        return Ok(LauncherNext::Continue);
    }
    if let Some(agent) = action.strip_prefix("Toggle tools ") {
        toggle_agent_routing_field(path, agent.trim(), RoutingField::Tools)?;
        return Ok(LauncherNext::Continue);
    }
    if let Some(agent) = action.strip_prefix("Toggle 1m ") {
        toggle_agent_routing_field(path, agent.trim(), RoutingField::MinContext)?;
        return Ok(LauncherNext::Continue);
    }
    if action == "Credits" {
        if let Err(err) = run_usage(parsed, env) {
            eprintln!("{}", term::err(&err));
        }
        return Ok(LauncherNext::Continue);
    }
    if action == "Agent onboard prompt…" {
        if let Err(err) = crate::onboard::run("onboard", parsed) {
            eprintln!("{}", term::err(&err));
        }
        return Ok(LauncherNext::Continue);
    }
    if action == "Login / add key" || action == "Login / sign in" {
        if let Err(err) = run_login(parsed, env) {
            eprintln!("{}", term::err(&err));
        }
        return Ok(LauncherNext::Continue);
    }
    Ok(LauncherNext::Continue)
}

fn switch_agent_model(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
    agent: &str,
) -> Result<LauncherNext, String> {
    if agent.is_empty() {
        return Ok(LauncherNext::Continue);
    }
    if !launcher_signed_in(path, parsed, env) {
        eprintln!("{}", term::err("Sign in first (Login / sign in)."));
        return Ok(LauncherNext::Continue);
    }
    match bind_agent_model(parsed, env, path, agent) {
        Ok(_) => {}
        Err(err) if err == "Cancelled." => {}
        Err(err) => eprintln!("{}", term::err(&err)),
    }
    Ok(LauncherNext::Continue)
}

fn bind_agent_model(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
    agent: &str,
) -> Result<i32, String> {
    let existing = load_config_if_present(path);
    let profile = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let key = resolve_api_key(&parsed.flags, env, profile);
    let base = resolve_base_url(&parsed.flags, profile);
    let models = fetch_models(&base, key.as_deref())?;
    let current = existing
        .as_ref()
        .and_then(|c| c.agent_binding(agent))
        .and_then(|b| b.default_model.clone())
        .or_else(|| profile.map(|p| p.default_model().to_string()));
    let id = pick_model(&models, current.as_deref(), &format!("{agent} model"))?;
    save_agent_model(path, agent, &id)
}

fn switch_agent_account(
    _parsed: &ParsedArgs,
    _env: &BTreeMap<String, String>,
    path: &PathBuf,
    agent: &str,
) -> Result<LauncherNext, String> {
    if agent.is_empty() {
        return Ok(LauncherNext::Continue);
    }
    match bind_agent_account(path, agent) {
        Ok(_) => {}
        Err(err) if err == "Cancelled." => {}
        Err(err) => eprintln!("{}", term::err(&err)),
    }
    Ok(LauncherNext::Continue)
}

fn bind_agent_account(path: &PathBuf, agent: &str) -> Result<i32, String> {
    let cfg = load_config_if_present(path).unwrap_or_default();
    let mut names: Vec<String> = cfg.profiles.keys().cloned().collect();
    names.sort();
    if names.is_empty() {
        return Err(no_key_error());
    }
    let current_name = cfg
        .agent_binding(agent)
        .and_then(|b| b.profile.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| cfg.active_profile.clone());
    let labels: Vec<String> = names
        .iter()
        .map(|n| {
            let p = cfg.profiles.get(n);
            let key = mask_api_key(p.and_then(|p| p.api_key.as_deref()));
            let mark = if n == &current_name { "  ●" } else { "" };
            format!("{n}  ·  {key}{mark}")
        })
        .collect();
    let current = names.iter().position(|n| n == &current_name);
    let idx = pick_list(
        &format!("Account for {agent}"),
        &["applies only to this agent · launch list stays put".into()],
        &labels,
        current,
    )?;
    let chosen = names[idx].clone();
    save_agent_account(path, agent, &chosen)
}

fn switch_agent_key(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
    agent: &str,
) -> Result<LauncherNext, String> {
    if agent.is_empty() {
        return Ok(LauncherNext::Continue);
    }
    if !launcher_signed_in(path, parsed, env) {
        eprintln!("{}", term::err("Sign in first (Login / sign in)."));
        return Ok(LauncherNext::Continue);
    }
    match bind_agent_key(parsed, env, path, agent) {
        Ok(_) => {}
        Err(err) if err == "Cancelled." => {}
        Err(err) => eprintln!("{}", term::err(&err)),
    }
    Ok(LauncherNext::Continue)
}

fn bind_agent_key(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
    agent: &str,
) -> Result<i32, String> {
    let (_keys_path, cfg, base, api_key) = keys_credential(parsed, env)?;
    let rows = crate::http::keys_newest_first(
        fetch_keys(&base, &api_key)?
            .into_iter()
            .filter(|r| r.active)
            .collect(),
    );
    if rows.is_empty() {
        return Err(hint("No active keys. Create one: {bin} keys create"));
    }
    let current_key = cfg
        .agent_binding(agent)
        .and_then(|b| b.api_key.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let current = rows
        .iter()
        .position(|r| is_active_key_row(&r.masked, current_key))
        .or(Some(0));
    let labels: Vec<String> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| key_pick_label(r, current == Some(i)))
        .collect();
    let idx = pick_list(
        &format!("API key for {agent}"),
        &[
            "applies only to this agent · newest first".into(),
            format!("current  {}", mask_api_key(current_key)),
        ],
        &labels,
        current,
    )?;
    let revealed = reveal_key(&base, &api_key, &rows[idx].hash)?;
    save_agent_key(path, agent, &revealed)
}

fn launch_agent_picker(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
) -> Result<LauncherNext, String> {
    if !launcher_signed_in(path, parsed, env) {
        eprintln!(
            "{}",
            term::warn("Not signed in — login first, or pass --key.")
        );
        if let Err(err) = run_login(parsed, env) {
            eprintln!("{}", term::err(&err));
            return Ok(LauncherNext::Continue);
        }
        if !launcher_signed_in(path, parsed, env) {
            return Ok(LauncherNext::Continue);
        }
    }
    let last = launcher_last_tool(path, parsed, env);
    let present = available_agents(env, |id| tool_command_for(path, id));
    if present.is_empty() {
        if let Err(err) = install_agent_dialog(path, env) {
            if err != "Cancelled." {
                eprintln!("{}", term::err(&err));
            }
        }
        return Ok(LauncherNext::Continue);
    }
    let labels: Vec<String> = present
        .iter()
        .map(|(id, label)| format!("{id}  —  {label}"))
        .collect();
    let current = present.iter().position(|(id, _)| *id == last.as_str());
    let idx = match term::pick("Launch coding agent", &labels, current) {
        Ok(i) => i,
        Err(err) if err == "Cancelled." => return Ok(LauncherNext::Continue),
        Err(err) => {
            eprintln!("{}", term::err(&err));
            return Ok(LauncherNext::Continue);
        }
    };
    let tool = present[idx].0;
    Ok(LauncherNext::Exit(run_launch(tool, parsed, env)?))
}

/// Bare `ar` / `anyr` opens the TUI on a terminal (or dump mode); pipes still get --help.
fn should_open_launcher(raw: &[String], interactive: bool, dump: bool) -> bool {
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

#[cfg(test)]
mod persist_login_tests {
    use super::*;
    use crate::config::{parse_config, serialize_config};

    #[test]
    fn relogin_preserves_relay_pairing_and_extra_fields() {
        // Simulates persist_login's profile-rebuild: stored fields must carry
        // over or every login drops relay pairing and unrecognized keys.
        let before = "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-old-key
    base_url: https://anyrouter.dev/api
    pinned_preset: \"@preset/coding-stack\"
    default_model: anthropic/claude-sonnet-4.6
    default_tool: codex
    claude_haiku: z/glm
    timeout_ms: 3000000
    relay_token: rk_device-token
    relay_device_id: dev_abc
    future_field: keep-me
";
        let cfg = parse_config(before);
        let stored = cfg.profiles.get("default");

        // The rebuild in persist_login: fresh default + carried-over fields.
        let mut profile = create_default_profile(DefaultProfileInput {
            api_key: Some("sk-ar-v1-new-key".into()),
            base_url: stored.map(|p| p.base_url().to_string()),
            preset: None,
            timeout_ms: None,
            default_model: stored.and_then(|p| p.default_model.clone()),
        });
        profile.management_key = None;
        if let Some(tool) = stored.and_then(|p| p.default_tool.clone()) {
            profile.default_tool = Some(tool);
        }
        if let Some(prev) = stored {
            for slot in [
                "claude_haiku",
                "claude_sonnet",
                "claude_opus",
                "claude_fable",
            ] {
                let value = match slot {
                    "claude_haiku" => prev.claude_haiku.clone(),
                    "claude_sonnet" => prev.claude_sonnet.clone(),
                    "claude_opus" => prev.claude_opus.clone(),
                    _ => prev.claude_fable.clone(),
                };
                if let Some(v) = value {
                    match slot {
                        "claude_haiku" => profile.claude_haiku = Some(v),
                        "claude_sonnet" => profile.claude_sonnet = Some(v),
                        "claude_opus" => profile.claude_opus = Some(v),
                        _ => profile.claude_fable = Some(v),
                    }
                }
            }
            profile.relay_token = prev.relay_token.clone();
            profile.relay_device_id = prev.relay_device_id.clone();
            profile.extra = prev.extra.clone();
        }

        let out = parse_config(&serialize_config(&upsert_profile(cfg, "default", profile)));
        let p = out.profiles.get("default").unwrap();
        assert_eq!(p.api_key.as_deref(), Some("sk-ar-v1-new-key"));
        assert_eq!(p.relay_token.as_deref(), Some("rk_device-token"));
        assert_eq!(p.relay_device_id.as_deref(), Some("dev_abc"));
        assert_eq!(
            p.default_model.as_deref(),
            Some("anthropic/claude-sonnet-4.6")
        );
        assert_eq!(p.default_tool.as_deref(), Some("codex"));
        assert_eq!(
            p.extra.get("future_field").and_then(|v| v.as_str()),
            Some("keep-me")
        );
    }
}

#[cfg(test)]
mod picker_catalog_tests {
    use super::*;
    use crate::http::CatalogModel;

    fn cm(id: &str, context_length: Option<i64>) -> CatalogModel {
        CatalogModel {
            id: id.into(),
            name: None,
            owned_by: None,
            context_length,
        }
    }

    #[test]
    fn pick_ids_pins_anyrouter_auto_and_does_not_dump_usage_order() {
        // Input order is a usage ranking (most-used first).
        let models = vec![
            cm("stealth/ox-alpha", Some(1_000_000)),
            cm("openai/gpt-5.4-mini", Some(128_000)),
            cm("anthropic/claude-sonnet-4.6", Some(200_000)),
            cm("auto", None),
        ];
        let ids = pick_ids(&models);
        assert_eq!(ids[0], "anyrouter/auto");
        assert_eq!(
            ids,
            vec![
                "anyrouter/auto",
                "anthropic/claude-sonnet-4.6",
                "openai/gpt-5.4-mini",
                "stealth/ox-alpha",
            ]
        );
        assert_ne!(ids[1], "stealth/ox-alpha", "must not lead with most-used");
    }

    #[test]
    fn pick_ids_empty_catalog_still_has_preset() {
        assert_eq!(pick_ids(&[]), vec!["anyrouter/auto"]);
    }

    #[test]
    fn model_pick_label_shows_preset_id_not_most_used() {
        let models = vec![cm("stealth/ox-alpha", Some(1_000_000))];
        let label = model_pick_label("anyrouter/auto", &models);
        assert_eq!(label, "anyrouter/auto");
        assert!(!label.contains("most used"), "{label}");
        assert!(!label.contains("stealth/ox-alpha"), "{label}");
        assert_eq!(
            model_pick_label("stealth/ox-alpha", &models),
            "stealth/ox-alpha  ·  1M"
        );
    }

    #[cfg(feature = "native")]
    #[test]
    fn picker_typeahead_selects_anyrouter_auto() {
        use crate::tui::{drive_picker, Action, Outcome, PickerState};
        let models = vec![
            cm("stealth/ox-alpha", Some(1_000_000)),
            cm("openai/gpt-5.4-mini", None),
        ];
        let (ids, labels) = picker_choice_list(&models);
        assert_eq!(ids[0], "anyrouter/auto");
        assert_eq!(labels[0], "anyrouter/auto");
        assert!(
            labels.iter().all(|l| !l.contains("most used")),
            "{labels:?}"
        );
        let mut state = PickerState::new("Default model", labels, Some(0));
        let typed: Vec<Action> = "anyrouter/auto".chars().map(Action::Char).collect();
        let out = drive_picker(&mut state, &typed);
        assert_eq!(out, Outcome::Continue);
        assert_eq!(state.filtered()[0].1, "anyrouter/auto");
        assert_eq!(state.apply(Action::Enter), Outcome::Selected(0));
        assert_eq!(ids[0], "anyrouter/auto");
    }

    #[cfg(feature = "native")]
    #[test]
    fn picker_dump_leads_with_preset_not_usage_ranking() {
        let models = vec![
            cm("stealth/ox-alpha", Some(1_000_000)),
            cm("openai/gpt-5.4-mini", None),
        ];
        let env = BTreeMap::new();
        let frame = render_model_picker_dump(&models, Some("auto"), &env);
        assert!(frame.contains("anyrouter/auto"), "{frame}");
        assert!(!frame.contains("most used"), "{frame}");
        let auto_at = frame.find("anyrouter/auto").expect("preset");
        let usage_at = frame.find("stealth/ox-alpha");
        if let Some(usage_at) = usage_at {
            assert!(
                auto_at < usage_at,
                "preset must appear before catalog dump:\n{frame}"
            );
        }
    }
}
