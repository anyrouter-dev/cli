use std::collections::BTreeMap;
use std::path::Path;

use crate::config::{get_active_profile, read_config, Config, Profile, DEFAULT_BASE_URL};
use crate::parse::{get_string_flag, FlagValue};

pub fn resolve_api_key(
    flags: &std::collections::HashMap<String, FlagValue>,
    env: &BTreeMap<String, String>,
    profile: Option<&Profile>,
) -> Option<String> {
    if let Some(key) = get_string_flag(flags, "key") {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(key) = env
        .get("ANYROUTER_API_KEY")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return Some(key.to_string());
    }
    profile
        .and_then(|p| p.api_key.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
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
