use std::collections::BTreeMap;
use std::path::Path;

use crate::config::{get_active_profile, read_config, Config, Profile, DEFAULT_BASE_URL};
use crate::parse::{get_string_flag, FlagValue};

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
}
