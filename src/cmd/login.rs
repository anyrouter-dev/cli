use std::collections::BTreeMap;

use crate::auth::acquire_api_key;
use crate::config::{
    create_default_profile, upsert_profile, write_config, DefaultProfileInput, DEFAULT_PROFILE,
};
use crate::http::validate_key;
use crate::key::{load_config_if_present, mask_api_key, resolve_base_url};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::term;

use crate::cmd::dispatch::{canonical_command, config_path};
use crate::cmd::keys::resolve_latest_key;

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
    let launch_tool =
        is_launch_command(&parsed.command).then(|| canonical_command(&parsed.command).to_string());
    apply_first_run_defaults(&mut cfg, &name, launch_tool);
    write_config(&cfg, &path)?;
    println!(
        "{}  {}  {}",
        term::ok("Signed in."),
        term::dim(&format!("via {source}")),
        term::dim(&format!("key {}", mask_api_key(Some(&key))))
    );
    println!("{}  {}", term::dim("Saved"), path.display());
    if login_next_hint(&parsed.command) {
        println!(
            "{}  {} claude",
            term::dim("Next"),
            crate::help::invoked_bin()
        );
    }
    Ok(0)
}

pub(crate) fn apply_first_run_defaults(
    cfg: &mut crate::config::Config,
    profile_name: &str,
    launch_tool: Option<String>,
) {
    if let Some(p) = cfg.profiles.get_mut(profile_name) {
        if p.default_tool.is_none() {
            p.default_tool = launch_tool.clone().or_else(|| Some("claude".into()));
        }
    }
    if cfg.last_tool.is_none() {
        cfg.last_tool = launch_tool.or_else(|| Some("claude".into()));
    }
}

pub(crate) fn is_launch_command(command: &str) -> bool {
    matches!(
        canonical_command(command),
        "claude" | "codex" | "grok" | "opencode" | "pi" | "pool"
    )
}

pub(crate) fn login_next_hint(command: &str) -> bool {
    matches!(canonical_command(command), "login" | "setup" | "auth")
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
    use crate::config::{parse_config, serialize_config, Config, Profile};

    #[test]
    fn first_run_defaults_to_claude_without_wizard() {
        // WHY: install → login → claude. No model/agent picker in between.
        let mut cfg = Config::default();
        cfg.profiles.insert("default".into(), Profile::default());
        apply_first_run_defaults(&mut cfg, "default", None);
        assert_eq!(cfg.last_tool.as_deref(), Some("claude"));
        assert_eq!(
            cfg.profiles
                .get("default")
                .and_then(|p| p.default_tool.as_deref()),
            Some("claude")
        );
        assert!(login_next_hint("login"));
        assert!(login_next_hint("auth"));
        assert!(!login_next_hint("menu"));
        assert!(!login_next_hint("claude"));
    }

    #[test]
    fn named_launch_pins_that_agent_and_relogin_keeps_it() {
        let mut cfg = Config::default();
        cfg.profiles.insert("default".into(), Profile::default());
        apply_first_run_defaults(&mut cfg, "default", Some("codex".into()));
        assert_eq!(cfg.last_tool.as_deref(), Some("codex"));
        apply_first_run_defaults(&mut cfg, "default", Some("claude".into()));
        assert_eq!(
            cfg.profiles
                .get("default")
                .and_then(|p| p.default_tool.as_deref()),
            Some("codex"),
            "relogin must not overwrite an existing default_tool"
        );
    }

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
