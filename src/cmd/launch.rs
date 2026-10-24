use std::collections::BTreeMap;

use crate::auth::acquire_api_key;
use crate::config::{write_config, DEFAULT_PROFILE};
use crate::http::fetch_models;
use crate::install::{ensure_tool_installed, Install};
use crate::key::{
    load_config_if_present, profile_for_agent, resolve_base_url, resolve_launch_api_key,
    resolve_launch_model,
};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::{
    apply_model_id_routing, apply_routing_env, build_tool_env, catalog_context_window,
    catalog_model_id, default_profile_for_env, display_model_id, effort_args_for, env_command_path,
    grok_user_home, is_auto_model, is_user_preset, is_virtual_preset, model_args_for,
    normalize_effort, opencode_major_version, opencode_standalone_args, prepare_grok_home,
    prepare_pi_wrapper, provider_args_for, render_dry_run, resolve_tool, spawn_child,
    BuildToolEnvInput, GROK_UNSET_ENV,
};
use crate::term;

use crate::cmd::decision::{chat_model_error, is_systemone_model};
use crate::cmd::dispatch::{catalog_lookup_enabled, config_path, yolo_for};
use crate::cmd::login::persist_login;
use crate::cmd::models::apply_claude_alias_flags;

pub(crate) struct ResolvedModel {
    id: String,
    context_window: Option<i64>,
    /// The catalog fetched to resolve `id`, kept so pi does not fetch it again.
    catalog: Option<Vec<crate::http::CatalogModel>>,
}

pub(crate) fn resolve_session_model(
    requested: &str,
    base: &str,
    key: Option<&str>,
    env: &BTreeMap<String, String>,
) -> ResolvedModel {
    let requested = catalog_model_id(requested);
    let id = if is_auto_model(&requested) {
        display_model_id(&requested)
    } else {
        requested
    };
    if is_virtual_preset(&id) || is_user_preset(&id) || !catalog_lookup_enabled(env) {
        // Virtual anyrouter/* stays the preset + floor suffix. Do not remap
        // onto a catalog SKU or inherit auto's 200k listing window.
        return ResolvedModel {
            id,
            context_window: None,
            catalog: None,
        };
    }
    let Ok(models) = fetch_models(base, key) else {
        return ResolvedModel {
            id,
            context_window: None,
            catalog: None,
        };
    };
    let context_window = catalog_context_window(&id, &models);
    ResolvedModel {
        id,
        context_window,
        catalog: Some(models),
    }
}

/// Put the agent's native skip-permission argv in `args`: after its
/// subcommand when the user runs one, else before the agent's own args.
/// Never twice: a native flag the user passed already wins.
fn add_yolo(tool_name: &str, args: &mut Vec<String>, agent_args_at: usize, quiet: bool) {
    let Some(yolo) = yolo_for(tool_name).filter(|y| !y.args.is_empty()) else {
        if !quiet {
            eprintln!(
                "{} {tool_name} has no flag to skip permission prompts; --yolo ignored.",
                term::dim("note:")
            );
        }
        return;
    };
    let first = yolo.args[0];
    let forwarded = &args[agent_args_at..];
    if forwarded
        .iter()
        .any(|a| a == first || a.starts_with(&format!("{first}=")))
    {
        return;
    }
    let at = yolo
        .after
        .and_then(|sub| {
            forwarded
                .iter()
                .position(|a| !a.starts_with('-'))
                .filter(|&i| forwarded[i] == sub)
        })
        .map_or(agent_args_at, |i| agent_args_at + i + 1);
    args.splice(at..at, yolo.args.iter().map(|a| a.to_string()));
}

/// Portable switches and each agent's own spelling, from its `--help`.
/// Empty: the agent has none, so the flag is dropped with a notice.
type PortableFlag = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

const PORTABLE_FLAGS: &[PortableFlag] = &[
    (
        "--skills-off",
        "skills",
        &[
            ("pi", "--no-skills"),
            ("claude", "--disable-slash-commands"),
        ],
    ),
    ("--tools-off", "tools", &[("pi", "--no-tools")]),
];

/// The agent's native flag for each `--skills-off` / `--tools-off` anyr
/// owns (registered in the launch flags like `--yolo`), or one stderr line
/// when the agent has none. Args after `--` never reach here.
/// Note: claude's `--disable-slash-commands` also disables slash commands.
fn map_portable_flags(tool_name: &str, parsed: &ParsedArgs, quiet: bool) -> Vec<String> {
    PORTABLE_FLAGS
        .iter()
        .filter(|(flag, _, _)| parsed.flag_true(&flag[2..]))
        .filter_map(|(flag, what, map)| {
            let native = map.iter().find(|(t, _)| *t == tool_name).map(|(_, n)| *n);
            if native.is_none() && !quiet {
                eprintln!(
                    "{} {tool_name} has no {what} switch; ignoring {flag}",
                    term::dim("note:")
                );
            }
            native.map(str::to_string)
        })
        .collect()
}

pub(crate) fn run_launch(
    tool_name: &str,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    launch(tool_name, parsed, env, None)
}

/// Headless one-shot run for `anyr delegate`: same gateway env as a normal
/// launch, but the agent gets the task through its non-interactive mode, the
/// saved config is left alone, and `extra_env` is added to the child.
pub(crate) struct Headless {
    pub(crate) args: Vec<String>,
    pub(crate) extra_env: Vec<(String, String)>,
}

pub(crate) fn launch(
    tool_name: &str,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    headless: Option<Headless>,
) -> Result<i32, String> {
    // `--preset <slug>` launches with the model id `@preset/<slug>`.
    let preset_args = crate::cmd::presets::launch_args(parsed)?;
    let preset_slug = preset_args.as_ref().map(|(_, slug)| slug.clone());
    let parsed = preset_args.as_ref().map_or(parsed, |(next, _)| next);
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let stored = existing
        .as_ref()
        .and_then(|c| profile_for_agent(c, &parsed.flags, env, tool_name));
    // Reject a known Decisions model before acquiring a key or spawning a chat
    // agent. The explicit flag path is important for non-interactive callers:
    // they should get the routing error, not a login prompt first.
    let guard_profile = stored.cloned().unwrap_or_else(|| {
        default_profile_for_env(Some(&resolve_base_url(&parsed.flags, None)), None)
    });
    let guard_model =
        resolve_launch_model(&parsed.flags, existing.as_ref(), &guard_profile, tool_name);
    if is_systemone_model(&guard_model) {
        return Err(chat_model_error(&guard_model, tool_name));
    }
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
    if let Some(slug) = &preset_slug {
        crate::cmd::presets::verify_for_launch(parsed, env, &base, &key, slug)?;
    }
    let mut profile = stored
        .cloned()
        .unwrap_or_else(|| default_profile_for_env(Some(&base), Some(&key)));
    profile.base_url = Some(base.clone());
    let aliases_changed = apply_claude_alias_flags(&mut profile, parsed);
    let tool = resolve_tool(existing.as_ref(), tool_name)?;
    let mut routing = existing
        .as_ref()
        .and_then(|c| c.agent_binding(tool_name))
        .map(|b| b.routing.clone())
        .unwrap_or_default();
    let requested = apply_model_id_routing(
        &resolve_launch_model(&parsed.flags, existing.as_ref(), &profile, tool_name),
        &mut routing,
    );
    let mut resolved = resolve_session_model(&requested, &base, Some(&key), env);
    let model = resolved.id;
    if is_systemone_model(&model) {
        return Err(chat_model_error(&model, tool_name));
    }
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
        min_context: routing.min_context,
        model_map: None,
    });
    apply_routing_env(&mut env_map, &routing, tool_name);
    // An `anyr` the agent runs itself must not update or print mid-session.
    env_map.insert(crate::autoupdate::ENV_LAUNCHER_CHILD.into(), "1".into());
    if tool_name == "pi" {
        let catalog = match resolved.catalog.take() {
            Some(models) => models,
            None => fetch_models(&base, Some(&key)).unwrap_or_default(),
        };
        prepare_pi_wrapper(&mut env_map, &path, &profile, &tool, &model, &catalog)?;
    }
    if tool_name == "grok" {
        let user_home = grok_user_home(&|k| env.get(k).cloned());
        prepare_grok_home(&mut env_map, &path, &key, user_home.as_deref())?;
    }
    let mut args = Vec::new();
    args.extend(effort_args_for(tool_name, effort.as_deref()));
    args.extend(provider_args_for(tool_name, &profile));
    args.extend(model_args_for(
        tool_name,
        &model,
        model_mode,
        resolved.context_window,
    ));
    // Wire the AnyRouter MCP gateway in when the workspace asks for it.
    let mcp_decision = crate::mcp_wiring::decide(&crate::mcp_wiring::DecideInput {
        tool: tool_name,
        no_mcp_flag: parsed.flag_true("no-mcp"),
        config: existing.as_ref(),
        base: &base,
        key: &key,
        cache_dir: path.parent(),
        dry_run: parsed.flag_true("dry-run"),
    });
    let mcp = crate::mcp_wiring::apply(
        &crate::mcp_wiring::ApplyInput {
            tool: tool_name,
            decision: &mcp_decision,
            key: &key,
            env,
            config_path: &path,
            dry_run: parsed.flag_true("dry-run"),
        },
        &mut args,
        &mut env_map,
    );
    let agent_args_at = args.len();
    // Native switches lead the agent's args: after a subcommand or a literal
    // `--` they would be rejected or become positionals.
    let portable = map_portable_flags(tool_name, parsed, term::machine_output(parsed, env));
    args.splice(agent_args_at..agent_args_at, portable);
    if let Some(h) = &headless {
        args.extend(h.args.clone());
        env_map.extend(h.extra_env.iter().cloned());
    }
    args.extend(parsed.passthrough.iter().cloned());
    if parsed.flag_true("yolo") || tool.extra_flag("yolo") {
        add_yolo(
            tool_name,
            &mut args,
            agent_args_at,
            term::machine_output(parsed, env),
        );
    }
    let unset: &[&str] = if tool_name == "grok" {
        GROK_UNSET_ENV
    } else {
        &[]
    };
    let command = get_string_flag(&parsed.flags, "command-path")
        .or_else(|| env_command_path(tool_name, env))
        .unwrap_or_else(|| tool.command.clone());
    if parsed.flag_true("dry-run") {
        if tool_name == "opencode" {
            if let Some(path) = crate::install::resolve_executable(&command) {
                args = opencode_standalone_args(&args, opencode_major_version(&path));
            }
        }
        println!("{}", render_dry_run(&command, &args, &env_map));
        println!("{}", mcp.summary);
        return Ok(0);
    }
    let install = if parsed.flag_true("install") {
        Install::Yes
    } else if term::agent_mode(parsed, env) {
        Install::Never
    } else {
        Install::Ask
    };
    if headless.is_some() {
        crate::api::confirm(
            parsed,
            &format!(
                "Run {tool_name} headlessly with model {model}? An agent run can make many model calls."
            ),
        )?;
        let resolved = ensure_tool_installed(tool_name, &command, install)?;
        if tool_name == "opencode" {
            args = opencode_standalone_args(&args, opencode_major_version(&resolved));
        }
        return Ok(spawn_child(&resolved, &args, &env_map, unset));
    }
    let resolved = ensure_tool_installed(tool_name, &command, install)?;
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
    // A preset launch is one-off: it never becomes the saved default model.
    let flag_model = get_string_flag(&parsed.flags, "model").filter(|_| preset_slug.is_none());
    if let Some(flag_model) = flag_model {
        let id = catalog_model_id(&flag_model);
        let stored = if is_auto_model(&id) { None } else { Some(id) };
        if let Some(p) = cfg.profiles.get_mut(&cfg.active_profile) {
            // Auto stays unset so the next launch keeps the anyrouter/auto preset.
            p.default_model = stored.clone();
        }
        cfg.agent_binding_mut(tool_name).default_model = stored;
        cfg.prune_empty_agents();
    }
    let _ = write_config(&cfg, &path);
    if tool_name == "opencode" {
        args = opencode_standalone_args(&args, opencode_major_version(&resolved));
    }
    Ok(spawn_child(&resolved, &args, &env_map, unset))
}
