use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::config::{write_config, Profile};
use crate::http::{fetch_models, format_models_list, CatalogModel};
use crate::install::KNOWN_AGENTS;
use crate::key::{
    load_config_if_present, mask_api_key, no_key_error, resolve_api_key, resolve_base_url,
};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::{
    canonical_tool, catalog_model_id, display_model_id, is_auto_model, session_model_label,
};
use crate::term;

use crate::cmd::dispatch::{catalog_lookup_enabled, config_path, hint, tui_wants_dump};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoutingField {
    Exacto,
    Tools,
    MinContext,
}

impl RoutingField {
    pub(crate) fn label(self) -> &'static str {
        match self {
            RoutingField::Exacto => "exacto",
            RoutingField::Tools => "tools",
            RoutingField::MinContext => "1M ctx",
        }
    }

    #[allow(dead_code)]
    pub(crate) fn action_kind(self) -> &'static str {
        match self {
            RoutingField::Exacto => "exacto",
            RoutingField::Tools => "tools",
            RoutingField::MinContext => "1m",
        }
    }
}

pub(crate) fn model_pick_label(id: &str, models: &[CatalogModel]) -> String {
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
pub(crate) fn pick_ids(models: &[CatalogModel]) -> Vec<String> {
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

pub(crate) fn picker_choice_list(models: &[CatalogModel]) -> (Vec<String>, Vec<String>) {
    let ids = pick_ids(models);
    let labels = ids.iter().map(|id| model_pick_label(id, models)).collect();
    (ids, labels)
}

/// Catalog rows for the inline picker. Live fetch is optional: the preset
/// `anyrouter/auto` is always selectable even when lookup is off or fails.
pub(crate) fn models_for_picker(
    base: &str,
    key: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Vec<CatalogModel> {
    if !catalog_lookup_enabled(env) {
        return Vec::new();
    }
    fetch_models(base, key).unwrap_or_default()
}

pub(crate) fn render_model_picker_dump(
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

pub(crate) fn pick_list(
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

pub(crate) fn pick_model(
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

pub(crate) fn set_model_slot(profile: &mut Profile, slot: &str, id: String) {
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

pub(crate) fn pick_claude_slot(profile: &Profile) -> Result<&'static str, String> {
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

pub(crate) fn slot_title(slot: &str) -> &'static str {
    match slot {
        "haiku" => "Haiku model",
        "sonnet" => "Sonnet model",
        "opus" => "Opus model",
        "fable" => "Fable model",
        _ => "Default model",
    }
}

pub(crate) fn slot_current<'a>(profile: &'a Profile, slot: &str) -> &'a str {
    match slot {
        "haiku" => profile.claude_haiku(),
        "sonnet" => profile.claude_sonnet(),
        "opus" => profile.claude_opus(),
        "fable" => profile.claude_fable(),
        _ => profile.default_model(),
    }
}

pub(crate) fn apply_claude_alias_flags(profile: &mut Profile, parsed: &ParsedArgs) -> bool {
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

pub(crate) fn known_model_id(models: &[CatalogModel], id: &str) -> bool {
    let id = catalog_model_id(id);
    is_auto_model(&id) || models.iter().any(|m| catalog_model_id(&m.id) == id)
}

pub(crate) fn save_model_slot(
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

pub(crate) fn flag_agent(parsed: &ParsedArgs) -> Option<String> {
    get_string_flag(&parsed.flags, "agent").map(|s| canonical_tool(&s).to_string())
}

pub(crate) fn known_agent(name: &str) -> Result<String, String> {
    let id = canonical_tool(name);
    if KNOWN_AGENTS.iter().any(|(k, _)| *k == id) {
        Ok(id.to_string())
    } else {
        Err(format!(
            "Unknown coding agent \"{name}\". Known: claude, codex, grok, opencode, pi, pool."
        ))
    }
}

pub(crate) fn save_agent_model(path: &PathBuf, agent: &str, id: &str) -> Result<i32, String> {
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

pub(crate) fn save_agent_account(
    path: &PathBuf,
    agent: &str,
    profile: &str,
) -> Result<i32, String> {
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

pub(crate) fn save_agent_key(path: &PathBuf, agent: &str, key: &str) -> Result<i32, String> {
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

pub(crate) fn palette_bind_detail(agent: &str) -> String {
    format!("for {agent}")
}

pub(crate) fn routing_toggle_detail(on: bool, agent: &str) -> String {
    format!("{} · for {agent}", if on { "on" } else { "off" })
}

pub(crate) fn toggle_agent_routing_field(
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

pub(crate) fn run_models(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
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
