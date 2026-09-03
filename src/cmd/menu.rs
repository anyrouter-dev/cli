use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::http::{fetch_keys, fetch_models, is_active_key_row, reveal_key};
use crate::install::{available_agents, ensure_tool_installed, missing_agents};
use crate::key::{
    load_config_if_present, mask_api_key, no_key_error, resolve_api_key, resolve_base_url,
};
use crate::parse::{get_string_flag, FlagValue, ParsedArgs};
use crate::spawn::session_model_label;
use crate::term;

use crate::cmd::config_tui::{
    config_account_actions, config_edit_row, run_config_tui, SettingKind,
};
#[cfg(feature = "native")]
use crate::cmd::dispatch::kick_credits_refresh;
use crate::cmd::dispatch::{
    config_path, hint, launcher_last_tool, launcher_uses_palette, persist_tool_command,
    pick_palette_action, tool_command_for, tui_dump_palette, tui_wants_dump, CreditsCache,
};

#[cfg(not(feature = "native"))]
use crate::cmd::dispatch::InlineEntry;
use crate::cmd::keys::{key_pick_label, keys_credential, run_keys, stored_api_key};
use crate::cmd::launch::run_launch;
use crate::cmd::login::run_login;
use crate::cmd::models::{
    palette_bind_detail, pick_list, pick_model, routing_toggle_detail, run_models,
    save_agent_account, save_agent_key, save_agent_model, toggle_agent_routing_field, RoutingField,
};
use crate::cmd::usage::run_usage;

pub(crate) fn agent_binding_detail(
    cfg: &crate::config::Config,
    id: &str,
    signed_in: bool,
) -> String {
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
pub(crate) fn launcher_palette(
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
pub(crate) fn launcher_palette(
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
pub(crate) fn push_inline_configure(
    entries: &mut Vec<InlineEntry>,
    agent: &str,
    cfg: &crate::config::Config,
) {
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

pub(crate) fn run_menu(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
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
    // Fullscreen-capable terminals get the command palette; dumb TTYs get
    // the same entries as an inline numbered prompt (Direction D fallback).
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
            match term::pick("anyr", &labels, Some(0)) {
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
pub(crate) enum LauncherNext {
    Continue,
    Exit(i32),
}

#[cfg(feature = "native")]
pub(crate) fn push_launch_entries(
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
pub(crate) fn push_agent_configure_entries(
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

pub(crate) fn install_agent_dialog(
    path: &PathBuf,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
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

pub(crate) fn launcher_signed_in(
    path: &PathBuf,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> bool {
    stored_api_key(parsed, env, path).is_some()
}

pub(crate) fn launcher_dispatch(
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

pub(crate) fn switch_agent_model(
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

pub(crate) fn bind_agent_model(
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

pub(crate) fn switch_agent_account(
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

pub(crate) fn bind_agent_account(path: &PathBuf, agent: &str) -> Result<i32, String> {
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

pub(crate) fn switch_agent_key(
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

pub(crate) fn bind_agent_key(
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

pub(crate) fn launch_agent_picker(
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
