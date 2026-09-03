use std::collections::BTreeMap;

use crate::auth::acquire_api_key;
use crate::config::{write_config, DEFAULT_PROFILE};
use crate::http::{fetch_models, most_used_model_id};
use crate::install::ensure_tool_installed;
use crate::key::{
    load_config_if_present, profile_for_agent, resolve_base_url, resolve_launch_api_key,
    resolve_launch_model,
};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::{
    apply_routing_env, build_tool_env, catalog_model_id, default_profile_for_env, effort_args_for,
    env_command_path, is_auto_model, model_args_for, normalize_effort, prepare_pi_wrapper,
    provider_args_for, render_dry_run, resolve_tool, spawn_child, BuildToolEnvInput,
};

use crate::cmd::dispatch::{catalog_lookup_enabled, config_path};
use crate::cmd::login::persist_login;
use crate::cmd::models::apply_claude_alias_flags;

pub(crate) struct ResolvedModel {
    id: String,
    context_window: Option<i64>,
}

/// Auto (unset / `auto` / `anyrouter/auto`) resolves to the catalog's most-used
/// model this week. A user-pinned id is kept. Failures keep the requested id.
pub(crate) fn resolve_session_model(
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

pub(crate) fn run_launch(
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
