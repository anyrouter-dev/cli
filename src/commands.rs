use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::auth::acquire_api_key;
use crate::config::{
    create_default_profile, resolve_config_path, set_active_profile, upsert_profile,
    valid_account_name, write_config, DefaultProfileInput, DEFAULT_PROFILE,
};
use crate::help::{command_help, root_help};
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
    build_tool_env, canonical_tool, default_profile_for_env, effort_args_for, env_command_path,
    model_args_for, normalize_effort, provider_args_for, render_dry_run, resolve_tool, spawn_child,
    BuildToolEnvInput,
};
use crate::term;
use crate::VERSION;

const LAUNCH_FLAGS: &[&str] = &[
    "model",
    "effort",
    "profile",
    "preset",
    "key",
    "management-key",
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

fn known_command(command: &str) -> bool {
    matches!(
        command,
        "setup"
            | "login"
            | "menu"
            | "models"
            | "chat"
            | "config"
            | "task"
            | "delegate"
            | "keys"
            | "whoami"
            | "status"
            | "audit"
            | "logout"
            | "account"
            | "usage"
            | "logs"
            | "transactions"
            | "skills"
            | "prompt"
            | "relay"
            | "byok"
            | "claude"
            | "cc"
            | "codex"
            | "grok"
            | "opencode"
            | "pool"
            | "poolside"
            | "pi"
            | "cursor"
            | "cline"
            | "windsurf"
            | "upgrade"
            | "update"
    )
}

fn canonical_command(command: &str) -> &str {
    match command {
        "update" => "upgrade",
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
            "management-key",
        ],
        "models" => &["profile", "config", "json", "key", "base-url", "pick"],
        "usage" => &["profile", "base-url", "config", "json", "key", "no-detail"],
        "whoami" => &["profile", "config", "json"],
        "config" => &["config", "json"],
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
            "management-key",
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
        "keys" => &[
            "profile",
            "base-url",
            "config",
            "management-key",
            "json",
            "yes",
        ],
        "audit" => &["profile", "config", "json", "launches", "tool", "limit"],
        "logout" => &["profile", "config"],
        "prompt" => &["base-url", "json"],
        "menu" => &[],
        "upgrade" => &["check", "channel", "fixture", "dry-run", "yes"],
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

pub fn run(argv: Vec<String>, env: HashMap<String, String>) -> i32 {
    let raw = if argv.first().map(String::as_str) == Some("--") {
        argv[1..].to_vec()
    } else {
        argv
    };
    let env: BTreeMap<String, String> = env.into_iter().collect();

    let parsed = match parse_cli_args(&raw) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let command = parsed.command.as_str();
    if command == "--version" || command == "-v" {
        println!("{VERSION}");
        return 0;
    }
    if command == "help" || command == "--help" || command == "-h" {
        print!("{}", root_help());
        return 0;
    }
    if raw.is_empty() {
        if term::is_interactive() {
            let empty = ParsedArgs {
                command: "menu".into(),
                flags: HashMap::new(),
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
        print!("{}", root_help());
        return 0;
    }
    if !known_command(command) {
        eprintln!("Unknown command \"{command}\". Run \"npx @anyr/cli --help\".");
        return 1;
    }
    if wants_help(&parsed) {
        if let Some(help) = command_help(command) {
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
        _ => stub(command),
    }
}

fn stub(command: &str) -> Result<i32, String> {
    Err(format!(
        "\"{command}\" is not yet implemented in native CLI. Pass --help, or use --key / ANYROUTER_API_KEY with login / spawn --dry-run."
    ))
}

fn config_path(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> PathBuf {
    resolve_config_path(get_string_flag(&parsed.flags, "config").as_deref(), env)
}

fn persist_login(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    key: &str,
    management_key: Option<String>,
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
    if let Some(mk) = management_key.or_else(|| get_string_flag(&parsed.flags, "management-key")) {
        profile.management_key = Some(mk);
    } else if let Some(prev) = stored.and_then(|p| p.management_key.clone()) {
        profile.management_key = Some(prev);
    }
    if let Some(tool) = stored.and_then(|p| p.default_tool.clone()) {
        profile.default_tool = Some(tool);
    }
    let mut cfg = upsert_profile(existing.unwrap_or_default(), &name, profile);
    cfg.active_profile = name.clone();
    if !parsed.flag_true("yes") && term::is_interactive() {
        if let Ok(models) = fetch_models(&base, Some(key)) {
            if let Ok(id) = pick_model(&models, None) {
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
    persist_login(
        parsed,
        env,
        &acquired.api_key,
        acquired.management_key,
        &acquired.source,
    )
}

fn pick_model(
    models: &[crate::http::CatalogModel],
    current: Option<&str>,
) -> Result<String, String> {
    if models.is_empty() {
        return Err("No models in catalog.".into());
    }
    let current_idx = current.and_then(|id| models.iter().position(|m| m.id == id));
    let query = term::prompt("Filter models (Enter lists top 30): ")?;
    let ids: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
    let ranked = term::rank_ids(&query, &ids);
    let shown: Vec<String> = ranked.into_iter().take(30).collect();
    if shown.is_empty() {
        return Err("No models matched.".into());
    }
    let shown_idx = current
        .and_then(|id| shown.iter().position(|s| s == id))
        .or(current_idx.filter(|_| query.is_empty()));
    let idx = term::pick("Default model", &shown, shown_idx)?;
    Ok(shown[idx].clone())
}

fn run_models(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let mut existing = load_config_if_present(&path);
    let profile = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let key = resolve_api_key(&parsed.flags, env, profile);
    let base = resolve_base_url(&parsed.flags, profile);
    let models = fetch_models(&base, key.as_deref())?;
    let sub = parsed.passthrough.first().map(String::as_str);
    if sub == Some("use") {
        let id = parsed
            .passthrough
            .get(1)
            .cloned()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                if term::is_interactive() {
                    pick_model(&models, profile.and_then(|p| p.default_model.as_deref())).ok()
                } else {
                    None
                }
            })
            .ok_or_else(|| "Usage: anyr models use <id>".to_string())?;
        if !models.iter().any(|m| m.id == id) && id != "auto" {
            return Err(format!("Unknown model \"{id}\". Run: anyr models"));
        }
        let mut cfg = existing.unwrap_or_default();
        let name = cfg.active_profile.clone();
        if let Some(p) = cfg.profiles.get_mut(&name) {
            p.default_model = Some(id.clone());
        } else {
            return Err(no_key_error());
        }
        write_config(&cfg, &path)?;
        println!(
            "{}  default model  {}",
            term::ok("Saved"),
            term::model_id(&id)
        );
        return Ok(0);
    }
    if parsed.flag_true("pick") && term::is_interactive() {
        let id = pick_model(&models, profile.and_then(|p| p.default_model.as_deref()))?;
        if let Some(cfg) = existing.as_mut() {
            let name = cfg.active_profile.clone();
            if let Some(p) = cfg.profiles.get_mut(&name) {
                p.default_model = Some(id.clone());
            }
            write_config(cfg, &path)?;
            println!(
                "{}  default model  {}",
                term::ok("Saved"),
                term::model_id(&id)
            );
            return Ok(0);
        }
        println!("{}", term::model_id(&id));
        return Ok(0);
    }
    let pinned = profile
        .and_then(|p| p.default_model.clone())
        .into_iter()
        .filter(|s| s != "auto")
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
            "management_key": if profile.management_key.as_ref().is_some_and(|s| !s.is_empty()) { "present" } else { "none" },
            "default_model": profile.default_model(),
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
        term::dim("management_key "),
        if profile
            .management_key
            .as_ref()
            .is_some_and(|s| !s.is_empty())
        {
            "present"
        } else {
            "none"
        }
    );
    println!(
        "{}  {}",
        term::dim("default_model  "),
        term::model_id(profile.default_model())
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

fn run_config(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let sub = parsed
        .passthrough
        .first()
        .map(String::as_str)
        .unwrap_or("get");
    match sub {
        "path" => {
            println!("{}", path.display());
            Ok(0)
        }
        "use" => {
            let name = parsed
                .passthrough
                .get(1)
                .cloned()
                .ok_or_else(|| "Usage: anyr config use <profile>".to_string())?;
            run_account_use(parsed, env, &name)
        }
        _ => {
            if parsed.flag_true("json") {
                let cfg = load_config_if_present(&path).unwrap_or_default();
                let payload = serde_json::json!({
                    "path": path.display().to_string(),
                    "active_profile": cfg.active_profile,
                    "accounts": cfg.profiles.keys().cloned().collect::<Vec<_>>(),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
                );
            } else {
                println!("{}", path.display());
            }
            Ok(0)
        }
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
        persist_login(
            parsed,
            env,
            &acquired.api_key,
            acquired.management_key,
            &acquired.source,
        )?;
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
    let tool = resolve_tool(existing.as_ref(), tool_name)?;
    let model = get_string_flag(&parsed.flags, "model")
        .unwrap_or_else(|| profile.default_model().to_string());
    let effort = normalize_effort(get_string_flag(&parsed.flags, "effort").as_deref())?;
    let model_mode = if model == "auto" { "auto" } else { "concrete" };
    let env_map = build_tool_env(BuildToolEnvInput {
        tool_name,
        tool: &tool,
        profile: &profile,
        api_key: &key,
        model: &model,
        effort: effort.as_deref(),
        context_window: None,
        model_map: None,
    });
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
        let _ = write_config(&cfg, &path);
    }
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
                    term::model_id(profile.default_model()),
                    mask_api_key(profile.api_key.as_deref())
                );
            }
            if cfg.profiles.is_empty() {
                println!("{}", term::dim("No accounts. Run: anyr login"));
            }
            Ok(0)
        }
        "use" => {
            let name = parsed
                .passthrough
                .get(1)
                .cloned()
                .ok_or_else(|| "Usage: anyr account use <name>".to_string())?;
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
                .ok_or_else(|| "Usage: anyr account rename <old> <new>".to_string())?;
            let new = parsed
                .passthrough
                .get(2)
                .cloned()
                .ok_or_else(|| "Usage: anyr account rename <old> <new>".to_string())?;
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
                .ok_or_else(|| "Usage: anyr account remove <name>".to_string())?;
            let mut cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
            if cfg.active_profile == name {
                return Err(format!(
                    "\"{name}\" is the active account. Switch first: anyr account use <other>"
                ));
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
) -> Result<
    (
        std::path::PathBuf,
        crate::config::Config,
        String,
        String,
        Option<String>,
    ),
    String,
> {
    let path = config_path(parsed, env);
    let cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
    let name =
        get_string_flag(&parsed.flags, "profile").unwrap_or_else(|| cfg.active_profile.clone());
    let profile = cfg
        .profiles
        .get(&name)
        .ok_or_else(|| format!("Profile \"{name}\" was not found in AnyRouter config."))?;
    let base = resolve_base_url(&parsed.flags, Some(profile));
    let api_key = resolve_api_key(&parsed.flags, env, Some(profile));
    let management =
        get_string_flag(&parsed.flags, "management-key").or_else(|| profile.management_key.clone());
    let credential = management
        .clone()
        .or_else(|| api_key.clone())
        .ok_or_else(|| "No stored credential. Run \"anyr login\" first.".to_string())?;
    Ok((path, cfg, base, credential, api_key))
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
            let (_path, _cfg, base, cred, api_key) = keys_credential(parsed, env)?;
            let rows = fetch_keys(&base, &cred)?;
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
                            "current": is_active_key_row(&r.masked, api_key.as_deref()),
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
                println!("{}", term::dim("No API keys. Create one: anyr keys create"));
                return Ok(0);
            }
            let name_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(4).max(4);
            for r in &rows {
                let marker = if is_active_key_row(&r.masked, api_key.as_deref()) {
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
                term::dim("* = key this profile uses · switch: anyr keys use")
            );
            Ok(0)
        }
        "create" => {
            let (path, mut cfg, base, cred, _api) = keys_credential(parsed, env)?;
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
            let (path, mut cfg, base, cred, api_key) = keys_credential(parsed, env)?;
            let rows: Vec<_> = fetch_keys(&base, &cred)?
                .into_iter()
                .filter(|r| r.active)
                .collect();
            if rows.is_empty() {
                return Err("No active keys. Create one: anyr keys create".into());
            }
            let hash_arg = parsed.passthrough.get(1).cloned();
            let row = if let Some(hash) = hash_arg {
                let matches: Vec<_> = rows
                    .iter()
                    .filter(|r| r.hash == hash || r.hash.starts_with(&hash))
                    .collect();
                match matches.as_slice() {
                    [one] => (*one).clone(),
                    [] => return Err(format!("No key matches \"{hash}\". See: anyr keys list")),
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
                    .position(|r| is_active_key_row(&r.masked, api_key.as_deref()));
                let idx = term::pick("Which key should this profile use?", &labels, current)?;
                rows[idx].clone()
            } else {
                return Err(
                    "Usage: anyr keys use <hash>   (interactive picker needs a terminal)".into(),
                );
            };
            let revealed = reveal_key(&base, &cred, &row.hash)?;
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
                "Usage: anyr keys revoke <hash>   (find hashes: anyr keys list)".to_string()
            })?;
            let (_path, _cfg, base, cred, _api) = keys_credential(parsed, env)?;
            let rows = fetch_keys(&base, &cred)?;
            let matches: Vec<_> = rows
                .iter()
                .filter(|r| r.hash == hash || r.hash.starts_with(&hash))
                .collect();
            let row = match matches.as_slice() {
                [one] => *one,
                [] => return Err(format!("No key matches \"{hash}\". See: anyr keys list")),
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

fn run_menu(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    if existing.is_none() && resolve_api_key(&parsed.flags, env, None).is_none() {
        return run_login(parsed, env);
    }
    let cfg = existing.unwrap_or_default();
    let profile = cfg.profiles.get(&cfg.active_profile);
    let last = cfg
        .last_tool
        .clone()
        .or_else(|| profile.and_then(|p| p.default_tool.clone()))
        .unwrap_or_else(|| "claude".into());
    println!("{}", term::bold("AnyRouter"));
    println!(
        "{}  {}  {}",
        term::dim("account"),
        term::accent(&cfg.active_profile),
        term::dim(&mask_api_key(profile.and_then(|p| p.api_key.as_deref())))
    );
    println!(
        "{}  {}",
        term::dim("model  "),
        term::model_id(profile.map(|p| p.default_model()).unwrap_or("auto"))
    );
    println!();
    let items = vec![
        format!("Launch {}", last),
        "Switch model".into(),
        "Switch account / key".into(),
        "Credits".into(),
        "Login / add key".into(),
        "Quit".into(),
    ];
    if !term::is_interactive() {
        println!("{}", items.join("\n"));
        return Ok(0);
    }
    let idx = term::pick("What next?", &items, Some(0))?;
    match idx {
        0 => run_launch(&last, parsed, env),
        1 => {
            let mut next = parsed.clone();
            next.command = "models".into();
            next.flags.insert("pick".into(), FlagValue::Bool(true));
            run_models(&next, env)
        }
        2 => {
            let names: Vec<String> = cfg.profiles.keys().cloned().collect();
            if names.len() > 1 {
                let current = names.iter().position(|n| n == &cfg.active_profile);
                let pick = term::pick("Account", &names, current)?;
                run_account_use(parsed, env, &names[pick])?;
            }
            let mut next = parsed.clone();
            next.command = "keys".into();
            next.passthrough = vec!["use".into()];
            run_keys(&next, env)
        }
        3 => run_usage(parsed, env),
        4 => run_login(parsed, env),
        _ => Ok(0),
    }
}
