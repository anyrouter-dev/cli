use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::config::{get_active_profile, read_config, Config, Profile, DEFAULT_BASE_URL};
use crate::parse::{get_string_flag, FlagValue};
use crate::spawn::canonical_tool;

/// Known non-secret fixture literals (mirrors the TS CLI's persistApiKey guard):
/// these exact values once overwrote a user's real key via a test fixture, so
/// they must never act as a live credential — treat them like "no key".
const DUMMY_KEYS: [&str; 3] = ["sk-ar-v1-test", "sk-ar-v1-testkey", "sk-ar-v1-test-key"];

pub fn is_dummy_key(value: &str) -> bool {
    let trimmed = value.trim();
    DUMMY_KEYS
        .iter()
        .any(|dummy| trimmed.eq_ignore_ascii_case(dummy))
}

pub fn resolve_api_key(
    flags: &std::collections::HashMap<String, FlagValue>,
    env: &BTreeMap<String, String>,
    profile: Option<&Profile>,
) -> Option<String> {
    if let Some(key) = get_string_flag(flags, "key") {
        let trimmed = key.trim();
        if !trimmed.is_empty() && !is_dummy_key(trimmed) {
            return Some(trimmed.to_string());
        }
    }
    if let Some(key) = env
        .get("ANYROUTER_API_KEY")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && !is_dummy_key(s))
    {
        return Some(key.to_string());
    }
    profile
        .and_then(|p| p.api_key.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !is_dummy_key(s))
        .map(str::to_string)
}

/// Profile used when launching `tool`: `--profile` / `ANYROUTER_PROFILE`, then
/// that agent's bound account, then the session `active_profile`.
pub fn profile_for_agent<'a>(
    config: &'a Config,
    flags: &HashMap<String, FlagValue>,
    env: &BTreeMap<String, String>,
    tool: &str,
) -> Option<&'a Profile> {
    if get_string_flag(flags, "profile").is_some()
        || env
            .get("ANYROUTER_PROFILE")
            .map(|s| s.trim())
            .is_some_and(|s| !s.is_empty())
    {
        return get_active_profile(config, get_string_flag(flags, "profile").as_deref(), env).ok();
    }
    let id = canonical_tool(tool);
    if let Some(name) = config
        .agent_binding(id)
        .and_then(|b| b.profile.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return config.profiles.get(name);
    }
    config.profiles.get(&config.active_profile)
}

/// Launch key for `tool`. `--key` / `ANYROUTER_API_KEY` win. A per-agent
/// `api_key` is used as stored — it does not fall back to the default profile.
pub fn resolve_launch_api_key(
    flags: &HashMap<String, FlagValue>,
    env: &BTreeMap<String, String>,
    config: Option<&Config>,
    tool: &str,
) -> Option<String> {
    if let Some(key) = resolve_api_key(flags, env, None) {
        return Some(key);
    }
    let cfg = config?;
    let id = canonical_tool(tool);
    if let Some(key) = cfg
        .agent_binding(id)
        .and_then(|b| b.api_key.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !is_dummy_key(s))
    {
        return Some(key.to_string());
    }
    profile_for_agent(cfg, flags, env, tool)
        .and_then(|p| p.api_key.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !is_dummy_key(s))
        .map(str::to_string)
}

/// Launch model for `tool`: `--model`, then the agent's bound id, then the
/// profile default. Does not invent catalog ids.
pub fn resolve_launch_model(
    flags: &HashMap<String, FlagValue>,
    config: Option<&Config>,
    profile: &Profile,
    tool: &str,
) -> String {
    if let Some(m) = get_string_flag(flags, "model") {
        return crate::spawn::catalog_model_id(&m);
    }
    let id = canonical_tool(tool);
    if let Some(m) = config
        .and_then(|c| c.agent_binding(id))
        .and_then(|b| b.default_model.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return crate::spawn::catalog_model_id(m);
    }
    profile.default_model().to_string()
}

pub fn resolve_base_url(
    flags: &std::collections::HashMap<String, FlagValue>,
    profile: Option<&Profile>,
) -> String {
    if let Some(url) = get_string_flag(flags, "base-url") {
        return url;
    }
    profile
        .map(|p| p.base_url().to_string())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

pub fn load_config_if_present(path: &Path) -> Option<Config> {
    if path.exists() {
        read_config(path).ok()
    } else {
        None
    }
}

pub fn active_profile<'a>(
    config: &'a Config,
    flags: &std::collections::HashMap<String, FlagValue>,
    env: &BTreeMap<String, String>,
) -> Result<&'a Profile, String> {
    let name = get_string_flag(flags, "profile");
    get_active_profile(config, name.as_deref(), env)
}

pub fn mask_api_key(key: Option<&str>) -> String {
    let k = key.unwrap_or("").trim();
    if k.is_empty() {
        return "<none>".into();
    }
    let prefix_len = if k.starts_with("sk-ar-v1-") { 14 } else { 8 };
    let prefix: String = k.chars().take(prefix_len.min(k.chars().count())).collect();
    let suffix: String = k
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{prefix}…{suffix}")
}

pub fn no_key_error() -> String {
    "No AnyRouter config and no key. Pass --key sk-ar-... or set ANYROUTER_API_KEY, \
or use --device/--device-code for headless environments, \
or run in an interactive terminal to log in."
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::FlagValue;

    fn profile_with_key(key: &str) -> Profile {
        Profile {
            api_key: Some(key.into()),
            ..Default::default()
        }
    }

    #[test]
    fn dummy_literals_are_detected_trimmed_and_case_insensitive() {
        for dummy in DUMMY_KEYS {
            assert!(is_dummy_key(dummy), "{dummy}");
            assert!(is_dummy_key(&format!("  {dummy}  ")), "{dummy}");
            assert!(is_dummy_key(&dummy.to_ascii_uppercase()), "{dummy}");
        }
        assert!(!is_dummy_key("sk-ar-v1-test-extra"));
        assert!(!is_dummy_key("sk-ar-v1-real0123456789"));
    }

    #[test]
    fn each_dummy_literal_resolves_to_none() {
        let env = BTreeMap::new();
        let mut flags = std::collections::HashMap::new();
        for dummy in DUMMY_KEYS {
            // Stored in the profile.
            let profile = profile_with_key(dummy);
            assert_eq!(
                resolve_api_key(&flags, &env, Some(&profile)),
                None,
                "{dummy}"
            );
            // Passed via --key.
            flags.insert("key".into(), FlagValue::Value(dummy.to_string()));
            assert_eq!(resolve_api_key(&flags, &env, None), None, "{dummy}");
            flags.clear();
            // Exported via ANYROUTER_API_KEY.
            let mut with_env = BTreeMap::new();
            with_env.insert("ANYROUTER_API_KEY".into(), dummy.to_string());
            assert_eq!(resolve_api_key(&flags, &with_env, None), None, "{dummy}");
        }
    }

    #[test]
    fn real_keys_still_resolve_from_every_source() {
        let real = "sk-ar-v1-real0123456789abcdef";
        let mut flags =
            std::collections::HashMap::from([("key".into(), FlagValue::Value(real.to_string()))]);
        assert_eq!(
            resolve_api_key(&flags, &BTreeMap::new(), None).as_deref(),
            Some(real)
        );
        flags.clear();
        let mut env = BTreeMap::new();
        env.insert("ANYROUTER_API_KEY".into(), real.to_string());
        assert_eq!(resolve_api_key(&flags, &env, None).as_deref(), Some(real));
        let profile = profile_with_key(real);
        assert_eq!(
            resolve_api_key(&flags, &BTreeMap::new(), Some(&profile)).as_deref(),
            Some(real)
        );
    }

    #[test]
    fn dummy_env_falls_through_to_real_profile_key() {
        let mut env = BTreeMap::new();
        env.insert("ANYROUTER_API_KEY".into(), "SK-AR-V1-TESTKEY".into());
        let profile = profile_with_key("sk-ar-v1-real0123456789");
        let flags = std::collections::HashMap::new();
        assert_eq!(
            resolve_api_key(&flags, &env, Some(&profile)).as_deref(),
            Some("sk-ar-v1-real0123456789")
        );
    }

    fn cfg_two_agents() -> Config {
        crate::config::parse_config(
            "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-default-key-aaaa
    default_model: auto
  work:
    api_key: sk-ar-v1-work-key-bbbb
    default_model: anthropic/claude-sonnet-4.6
agents:
  claude:
    api_key: sk-ar-v1-claude-key-cccc
    default_model: stealth/ox-alpha
  grok:
    profile: work
",
        )
    }

    #[test]
    fn launch_key_uses_agent_key_not_default_profile() {
        let cfg = cfg_two_agents();
        let flags = HashMap::new();
        let env = BTreeMap::new();
        assert_eq!(
            resolve_launch_api_key(&flags, &env, Some(&cfg), "claude").as_deref(),
            Some("sk-ar-v1-claude-key-cccc")
        );
        assert_eq!(
            resolve_launch_api_key(&flags, &env, Some(&cfg), "grok").as_deref(),
            Some("sk-ar-v1-work-key-bbbb")
        );
        assert_eq!(
            resolve_launch_api_key(&flags, &env, Some(&cfg), "codex").as_deref(),
            Some("sk-ar-v1-default-key-aaaa")
        );
    }

    #[test]
    fn launch_model_uses_agent_id_then_profile_default() {
        let cfg = cfg_two_agents();
        let flags = HashMap::new();
        let claude_profile = profile_for_agent(&cfg, &flags, &BTreeMap::new(), "claude").unwrap();
        let grok_profile = profile_for_agent(&cfg, &flags, &BTreeMap::new(), "grok").unwrap();
        assert_eq!(
            resolve_launch_model(&flags, Some(&cfg), claude_profile, "claude"),
            "stealth/ox-alpha"
        );
        assert_eq!(
            resolve_launch_model(&flags, Some(&cfg), grok_profile, "grok"),
            "anthropic/claude-sonnet-4.6"
        );
        let mut with_flag = HashMap::new();
        with_flag.insert(
            "model".into(),
            FlagValue::Value("stealth/ox-alpha[1m]".into()),
        );
        assert_eq!(
            resolve_launch_model(&with_flag, Some(&cfg), claude_profile, "claude"),
            "stealth/ox-alpha"
        );
    }
}
