use std::collections::BTreeMap;

use crate::http::{fetch_credits, format_usage_report};
use crate::key::{load_config_if_present, no_key_error, resolve_api_key, resolve_base_url};
use crate::parse::ParsedArgs;

use crate::cmd::dispatch::config_path;

pub(crate) fn run_usage(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let profile = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let key = resolve_api_key(&parsed.flags, env, profile).ok_or_else(no_key_error)?;
    let base = resolve_base_url(&parsed.flags, profile);
    let credits = fetch_credits(&base, &key)?;
    print!(
        "{}",
        format_usage_report(&credits, parsed.flag_true("json"))
    );
    Ok(0)
}
