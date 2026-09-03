use std::collections::BTreeMap;

use crate::auth::acquire_api_key;
use crate::config::{
    create_default_profile, upsert_profile, write_config, DefaultProfileInput, DEFAULT_PROFILE,
};
use crate::http::validate_key;
use crate::key::{load_config_if_present, mask_api_key, resolve_base_url};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::term;

use crate::cmd::dispatch::config_path;
use crate::cmd::keys::resolve_latest_key;
use crate::cmd::models::{models_for_picker, pick_model, set_model_slot};

pub(crate) fn persist_login(
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

pub(crate) fn run_login(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let stored = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let base = resolve_base_url(&parsed.flags, stored);
    let acquired = acquire_api_key(&parsed.flags, env, &base, Some("cli"))?;
    persist_login(parsed, env, &acquired.api_key, &acquired.source)
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
