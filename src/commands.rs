use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::auth::acquire_api_key;
use crate::config::{
    create_default_profile, resolve_config_path, set_active_profile, upsert_profile,
    valid_account_name, write_config, DefaultProfileInput, Profile, DEFAULT_PROFILE,
};
use crate::help::{command_help, resolve_bin, root_help, set_invoked_bin};
use crate::http::{
    create_key, delete_key, fetch_credits, fetch_keys, fetch_models, format_models_list,
    format_usage_report, is_active_key_row, reveal_key, validate_key,
};
use crate::install::ensure_tool_installed;
use crate::key::{
    load_config_if_present, mask_api_key, no_key_error, resolve_api_key, resolve_base_url,
};
use crate::parse::{get_string_flag, parse_cli_args, FlagValue, ParsedArgs};
use crate::spawn::{
    build_tool_env, canonical_tool, default_profile_for_env, display_model_id, effort_args_for,
    env_command_path, is_auto_model, model_args_for, normalize_effort, prepare_pi_wrapper,
    provider_args_for, render_dry_run, resolve_tool, spawn_child, BuildToolEnvInput,
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

#[cfg(feature = "native")]
fn tui_menu_select(
    title: &str,
    header: Vec<String>,
    items: Vec<String>,
) -> Result<Option<usize>, String> {
    crate::tui::run_menu_select(title, header, items)
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

#[cfg(feature = "native")]
fn tui_dump_menu(
    title: &str,
    header: Vec<String>,
    items: Vec<String>,
    env: &BTreeMap<String, String>,
) -> String {
    crate::tui::dump_menu_select(title, header, items, crate::tui::dump_cols(env))
}

#[cfg(not(feature = "native"))]
fn tui_dump_menu(
    title: &str,
    header: Vec<String>,
    items: Vec<String>,
    _env: &BTreeMap<String, String>,
) -> String {
    let mut lines = vec![format!("▲ {title}")];
    lines.extend(header);
    lines.extend(items);
    lines.push(String::new());
    lines.join("\n")
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
        | "pool" | "pi" | "upgrade" | "onboard" | "impl" | "plan" | "fix" | "deploy" | "cp" => {
            CmdKind::Implemented
        }
        "cursor" | "cline" | "windsurf" => CmdKind::HelpOnly,
        "chat" | "task" | "delegate" | "audit" | "logs" | "transactions" | "skills" | "prompt"
        | "relay" | "byok" => CmdKind::Stub,
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
            "fable",
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
        "keys" => &["profile", "base-url", "config", "json", "yes"],
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
    let name = get_string_flag(&parsed.flags, "profile").unwrap_or_else(|| {
        existing
            .as_ref()
            .map(|c| c.active_profile.clone())
            .unwrap_or_else(|| DEFAULT_PROFILE.into())
    });
    let timeout = get_string_flag(&parsed.flags, "timeout").and_then(|s| s.parse().ok());
    let mut profile = create_default_profile(DefaultProfileInput {
        api_key: Some(key.to_string()),
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
    let mut cfg = upsert_profile(existing.unwrap_or_default(), &name, profile);
    cfg.active_profile = name.clone();
    if !parsed.flag_true("yes") && term::is_interactive() {
        if let Ok(models) = fetch_models(&base, Some(key)) {
            if let Ok(id) = pick_model(&models, None, "Default model") {
                if let Some(p) = cfg.profiles.get_mut(&name) {
                    p.default_model = Some(id);
                }
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
        term::dim(&format!("key {}", mask_api_key(Some(key))))
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

fn model_pick_label(id: &str) -> String {
    if is_auto_model(id) {
        "anyrouter/auto  ·  smart pick".into()
    } else {
        id.to_string()
    }
}

fn pick_ids(models: &[crate::http::CatalogModel]) -> Vec<String> {
    let mut ids = Vec::new();
    ids.push("anyrouter/auto".into());
    for model in models {
        let id = display_model_id(&model.id).to_string();
        if !ids.iter().any(|existing| existing == &id) {
            ids.push(id);
        }
    }
    ids
}

fn pick_model(
    models: &[crate::http::CatalogModel],
    current: Option<&str>,
    title: &str,
) -> Result<String, String> {
    let ids = pick_ids(models);
    if ids.is_empty() {
        return Err("No models in catalog.".into());
    }
    let current_id = current.map(display_model_id);
    let query = term::prompt("Filter models (Enter lists top 30): ")?;
    let ranked = term::rank_ids(&query, &ids);
    let shown: Vec<String> = ranked.into_iter().take(30).collect();
    if shown.is_empty() {
        return Err("No models matched.".into());
    }
    let labels: Vec<String> = shown.iter().map(|id| model_pick_label(id)).collect();
    let shown_idx = current_id.and_then(|id| shown.iter().position(|s| s == id));
    let idx = term::pick(title, &labels, shown_idx)?;
    Ok(shown[idx].clone())
}

fn set_model_slot(profile: &mut Profile, slot: &str, id: String) {
    let id = display_model_id(&id).to_string();
    match slot {
        "haiku" => profile.claude_haiku = Some(id),
        "sonnet" => profile.claude_sonnet = Some(id),
        "opus" => profile.claude_opus = Some(id),
        "fable" => profile.claude_fable = Some(id),
        _ => profile.default_model = Some(id),
    }
}

fn pick_claude_slot(profile: &Profile) -> Result<&'static str, String> {
    let items = vec![
        format!("Default  ·  {}", display_model_id(profile.default_model())),
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

fn known_model_id(models: &[crate::http::CatalogModel], id: &str) -> bool {
    is_auto_model(id) || models.iter().any(|m| m.id == id)
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
    println!("{}  {}  {}", term::ok("Saved"), label, term::model_id(id));
    Ok(0)
}

fn run_models(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let profile = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let key = resolve_api_key(&parsed.flags, env, profile);
    let base = resolve_base_url(&parsed.flags, profile);
    let models = fetch_models(&base, key.as_deref())?;
    let sub = parsed.passthrough.first().map(String::as_str);
    let flag_slots = [
        ("haiku", get_string_flag(&parsed.flags, "haiku")),
        ("sonnet", get_string_flag(&parsed.flags, "sonnet")),
        ("opus", get_string_flag(&parsed.flags, "opus")),
        ("fable", get_string_flag(&parsed.flags, "fable")),
    ];
    let has_alias_flags = flag_slots.iter().any(|(_, v)| v.is_some());
    if sub == Some("use") || has_alias_flags {
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
            save_model_slot(existing.clone(), &path, slot, &display_model_id(&id))?;
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
                display_model_id(&id),
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
            display_model_id(&id.1),
        );
    }
    if parsed.flag_true("pick") && term::is_interactive() {
        let slot = profile
            .map(pick_claude_slot)
            .transpose()?
            .unwrap_or("default");
        let current = profile.map(|p| slot_current(p, slot).to_string());
        let id = pick_model(&models, current.as_deref(), slot_title(slot))?;
        return save_model_slot(existing, &path, slot, display_model_id(&id));
    }
    let pinned = profile
        .map(|p| display_model_id(p.default_model()).to_string())
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
        term::model_id(display_model_id(profile.default_model()))
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
        term::model_id(display_model_id(
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
    let items = vec![
        "Switch key".into(),
        "Switch account".into(),
        "Switch model".into(),
        "Credits".into(),
        "Sign in".into(),
        "Log out".into(),
        "Done".into(),
    ];
    if tui_wants_dump(parsed, env) {
        let header = config_tui_header(&path);
        print!("{}", tui_dump_menu("Config", header, items, env));
        return Ok(0);
    }
    loop {
        let header = config_tui_header(&path);
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
            display_model_id(profile.map(|p| p.default_model()).unwrap_or("auto"))
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

fn run_launch(
    tool_name: &str,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let stored = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let key = if let Some(key) = resolve_api_key(&parsed.flags, env, stored) {
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
        .and_then(|c| c.profiles.get(&c.active_profile));
    let base = resolve_base_url(&parsed.flags, stored);
    let mut profile = stored
        .cloned()
        .unwrap_or_else(|| default_profile_for_env(Some(&base), Some(&key)));
    profile.base_url = Some(base);
    let aliases_changed = apply_claude_alias_flags(&mut profile, parsed);
    let tool = resolve_tool(existing.as_ref(), tool_name)?;
    let model = get_string_flag(&parsed.flags, "model")
        .unwrap_or_else(|| profile.default_model().to_string());
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
        context_window: None,
        model_map: None,
    });
    if tool_name == "pi" {
        prepare_pi_wrapper(&mut env_map, &path, &profile, &tool, &model)?;
    }
    let mut args = Vec::new();
    args.extend(effort_args_for(tool_name, effort.as_deref()));
    args.extend(provider_args_for(tool_name, &profile));
    args.extend(model_args_for(tool_name, &model, model_mode));
    args.extend(parsed.passthrough.clone());
    let command = get_string_flag(&parsed.flags, "command-path")
        .or_else(|| env_command_path(tool_name, env))
        .unwrap_or_else(|| tool.command.clone());
    if parsed.flag_true("dry-run") {
        println!("{}", render_dry_run(&command, &args, &env_map));
        return Ok(0);
    }
    let resolved = ensure_tool_installed(tool_name, &command, parsed.flag_true("install"))?;
    if let Some(mut cfg) = existing.clone() {
        cfg.last_tool = Some(tool_name.to_string());
        if aliases_changed {
            if let Some(p) = cfg.profiles.get_mut(&cfg.active_profile) {
                p.claude_haiku = profile.claude_haiku.clone();
                p.claude_sonnet = profile.claude_sonnet.clone();
                p.claude_opus = profile.claude_opus.clone();
                p.claude_fable = profile.claude_fable.clone();
            }
        }
        let _ = write_config(&cfg, &path);
    }
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
                    term::model_id(display_model_id(profile.default_model())),
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
            let rows = fetch_keys(&base, &api_key)?;
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
            let rows: Vec<_> = fetch_keys(&base, &api_key)?
                .into_iter()
                .filter(|r| r.active)
                .collect();
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
                let labels: Vec<String> = rows
                    .iter()
                    .map(|r| format!("{} · {}", r.name, r.masked))
                    .collect();
                let current = rows
                    .iter()
                    .position(|r| is_active_key_row(&r.masked, Some(&api_key)));
                let idx = term::pick("Which key should this profile use?", &labels, current)?;
                rows[idx].clone()
            } else {
                return Err(hint(
                    "Usage: {bin} keys use <hash>   (interactive picker needs a terminal)",
                ));
            };
            let revealed = reveal_key(&base, &api_key, &row.hash)?;
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

fn run_menu(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let dumping = tui_wants_dump(parsed, env);

    if dumping {
        let (header, items) = launcher_frame(&path, parsed, env);
        print!("{}", tui_dump_menu("AnyRouter", header, items, env));
        return Ok(0);
    }

    if !term::is_interactive() {
        let (_, items) = launcher_frame(&path, parsed, env);
        println!("{}", items.join("\n"));
        return Ok(0);
    }

    // Loop until Quit or a coding-agent launch takes over the process.
    loop {
        let (header, items) = launcher_frame(&path, parsed, env);
        let Some(idx) = tui_menu_select("AnyRouter", header, items.clone())? else {
            return Ok(0);
        };
        let action = items.get(idx).map(String::as_str).unwrap_or("Quit");
        match launcher_dispatch(action, parsed, env, &path)? {
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

const LAUNCH_AGENTS: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("grok", "Grok Build"),
    ("opencode", "OpenCode"),
    ("pi", "Pi"),
    ("pool", "Poolside"),
];

fn launcher_last_tool(path: &PathBuf, parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> String {
    let cfg = load_config_if_present(path).unwrap_or_default();
    let profile = cfg.profiles.get(&cfg.active_profile);
    cfg.last_tool
        .clone()
        .or_else(|| profile.and_then(|p| p.default_tool.clone()))
        .or_else(|| get_string_flag(&parsed.flags, "tool"))
        .unwrap_or_else(|| {
            let _ = env;
            "claude".into()
        })
}

fn launcher_signed_in(path: &PathBuf, parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> bool {
    stored_api_key(parsed, env, path).is_some()
}

fn launcher_frame(
    path: &PathBuf,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> (Vec<String>, Vec<String>) {
    let cfg = load_config_if_present(path).unwrap_or_default();
    let profile = cfg.profiles.get(&cfg.active_profile);
    let signed_in = launcher_signed_in(path, parsed, env);
    let last = launcher_last_tool(path, parsed, env);
    let header = vec![
        format!(
            "account  {}  {}",
            cfg.active_profile,
            if signed_in {
                mask_api_key(profile.and_then(|p| p.api_key.as_deref()))
            } else {
                "(not signed in)".into()
            }
        ),
        format!(
            "model    {}",
            display_model_id(profile.map(|p| p.default_model()).unwrap_or("auto"))
        ),
        format!("agent    {last}"),
    ];

    let mut items = Vec::new();
    if signed_in {
        items.push(format!("Launch {last}"));
        items.push("Launch coding agent…".into());
        items.push("Config".into());
        items.push("Switch model".into());
        items.push("Switch account / key".into());
        items.push("Credits".into());
        items.push("Agent onboard prompt…".into());
        items.push("Login / add key".into());
    } else {
        items.push("Login / sign in".into());
        items.push("Launch coding agent…".into());
        items.push("Config".into());
        items.push("Agent onboard prompt…".into());
    }
    items.push("Quit".into());
    (header, items)
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
    if action == "Switch model" {
        if !launcher_signed_in(path, parsed, env) {
            eprintln!("{}", term::err("Sign in first (Login / sign in)."));
            return Ok(LauncherNext::Continue);
        }
        let mut next = parsed.clone();
        next.command = "models".into();
        next.flags.insert("pick".into(), FlagValue::Bool(true));
        if let Err(err) = run_models(&next, env) {
            eprintln!("{}", term::err(&err));
        }
        return Ok(LauncherNext::Continue);
    }
    if action == "Switch account / key" {
        let cfg = load_config_if_present(path).unwrap_or_default();
        let names: Vec<String> = cfg.profiles.keys().cloned().collect();
        if names.len() > 1 {
            let current = names.iter().position(|n| n == &cfg.active_profile);
            match term::pick("Account", &names, current) {
                Ok(pick) => {
                    if let Err(err) = run_account_use(parsed, env, &names[pick]) {
                        eprintln!("{}", term::err(&err));
                    }
                }
                Err(err) => {
                    if err != "Cancelled." {
                        eprintln!("{}", term::err(&err));
                    }
                }
            }
        }
        let mut next = parsed.clone();
        next.command = "keys".into();
        next.passthrough = vec!["use".into()];
        if let Err(err) = run_keys(&next, env) {
            eprintln!("{}", term::err(&err));
        }
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

fn launch_agent_picker(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
) -> Result<LauncherNext, String> {
    if !launcher_signed_in(path, parsed, env) {
        eprintln!("{}", term::warn("Not signed in — login first, or pass --key."));
        if let Err(err) = run_login(parsed, env) {
            eprintln!("{}", term::err(&err));
            return Ok(LauncherNext::Continue);
        }
        if !launcher_signed_in(path, parsed, env) {
            return Ok(LauncherNext::Continue);
        }
    }
    let last = launcher_last_tool(path, parsed, env);
    let labels: Vec<String> = LAUNCH_AGENTS
        .iter()
        .map(|(id, label)| format!("{id}  —  {label}"))
        .collect();
    let current = LAUNCH_AGENTS.iter().position(|(id, _)| *id == last.as_str());
    let idx = match term::pick("Launch coding agent", &labels, current) {
        Ok(i) => i,
        Err(err) if err == "Cancelled." => return Ok(LauncherNext::Continue),
        Err(err) => {
            eprintln!("{}", term::err(&err));
            return Ok(LauncherNext::Continue);
        }
    };
    let tool = LAUNCH_AGENTS[idx].0;
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
