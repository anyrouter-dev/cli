use std::collections::{BTreeMap, HashMap};
use std::io::{self, IsTerminal};
use std::path::PathBuf;

use crate::config::{
    create_default_profile, resolve_config_path, upsert_profile, write_config, DefaultProfileInput,
    DEFAULT_PROFILE,
};
use crate::help::{command_help, root_help};
use crate::http::{
    fetch_credits, fetch_models, format_models_list, format_usage_report, validate_key,
};
use crate::key::{
    load_config_if_present, mask_api_key, no_key_error, resolve_api_key, resolve_base_url,
};
use crate::parse::{get_string_flag, parse_cli_args, FlagValue, ParsedArgs};
use crate::spawn::{
    build_tool_env, canonical_tool, default_profile_for_env, effort_args_for, env_command_path,
    model_args_for, normalize_effort, provider_args_for, render_dry_run, resolve_tool, spawn_child,
    BuildToolEnvInput,
};
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
        "models" => &["profile", "config", "json", "key", "base-url"],
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
        "transactions" => &["profile", "base-url", "config", "json", "key", "limit", "type"],
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
    parsed.flag_true("help") || parsed.passthrough.iter().any(|a| a == "-h" || a == "--help")
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
    if command == "help" || command == "--help" || command == "-h" || raw.is_empty() {
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
        "models" => run_models(parsed, env),
        "usage" => run_usage(parsed, env),
        "whoami" => run_whoami(parsed, env),
        "config" => run_config(parsed, env),
        "account" => {
            if parsed.passthrough.first().map(String::as_str) == Some("list")
                || parsed.passthrough.is_empty()
            {
                run_whoami(parsed, env)
            } else {
                stub(command)
            }
        }
        "claude" | "codex" | "grok" | "opencode" | "pool" | "pi" => run_launch(command, parsed, env),
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

fn run_login(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let profile = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let Some(key) = resolve_api_key(&parsed.flags, env, profile) else {
        if io::stdin().is_terminal() && io::stdout().is_terminal() && !parsed.flag_true("yes") {
            return Err("not yet in native CLI; pass --key or set ANYROUTER_API_KEY".into());
        }
        return Err(no_key_error());
    };
    let base = resolve_base_url(&parsed.flags, profile);
    validate_key(&base, &key)?;
    let name = get_string_flag(&parsed.flags, "profile").unwrap_or_else(|| DEFAULT_PROFILE.into());
    let timeout = get_string_flag(&parsed.flags, "timeout").and_then(|s| s.parse().ok());
    let profile = create_default_profile(DefaultProfileInput {
        api_key: Some(key),
        base_url: Some(base),
        preset: get_string_flag(&parsed.flags, "preset"),
        timeout_ms: timeout,
        default_model: None,
    });
    let cfg = upsert_profile(existing.unwrap_or_default(), &name, profile);
    write_config(&cfg, &path)?;
    println!("Signed in. Config: {}", path.display());
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
    let (stdout, _) = format_models_list(&models, &[], None, parsed.flag_true("json"));
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
    print!("{}", format_usage_report(&credits, parsed.flag_true("json")));
    Ok(0)
}

fn run_whoami(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let Some(cfg) = load_config_if_present(&path) else {
        return Err(no_key_error());
    };
    let name = cfg.active_profile.clone();
    let key = cfg.profiles.get(&name).and_then(|p| p.api_key.as_deref());
    println!("active account  {name}");
    println!("config          {}", path.display());
    println!("api_key         {}", mask_api_key(key));
    let _ = parsed;
    Ok(0)
}

fn run_config(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let sub = parsed.passthrough.first().map(String::as_str).unwrap_or("get");
    match sub {
        "path" => {
            println!("{}", path.display());
            Ok(0)
        }
        _ => {
            println!("{}", path.display());
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
    let Some(key) = resolve_api_key(&parsed.flags, env, stored) else {
        return Err(no_key_error());
    };
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
    Ok(spawn_child(&command, &args, &env_map))
}
