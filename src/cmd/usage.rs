use std::collections::BTreeMap;

use crate::http::{fetch_credits, format_usage_report};
use crate::key::{
    load_config_if_present, mask_api_key, no_key_error, resolve_api_key, resolve_base_url,
};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::{display_model_id, session_model_label};
use crate::term;

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

pub(crate) fn run_whoami(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let Some(cfg) = load_config_if_present(&path) else {
        return Err(no_key_error());
    };
    let name = get_string_flag(&parsed.flags, "profile")
        .or_else(|| env.get("ANYROUTER_PROFILE").cloned())
        .unwrap_or_else(|| cfg.active_profile.clone());
    let profile = cfg
        .profiles
        .get(&name)
        .ok_or_else(|| format!("Profile \"{name}\" was not found in AnyRouter config."))?;
    let key = resolve_api_key(&parsed.flags, env, Some(profile));
    if parsed.flag_true("json") {
        let payload = serde_json::json!({
            "active_account": name,
            "config": path.display().to_string(),
            "api_key": mask_api_key(key.as_deref()),
            "default_model": display_model_id(profile.default_model()),
            "claude_haiku": profile.claude_haiku(),
            "claude_sonnet": profile.claude_sonnet(),
            "claude_opus": profile.claude_opus(),
            "claude_fable": profile.claude_fable(),
            "default_tool": profile.default_tool,
            "base_url": profile.base_url(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
        );
        return Ok(0);
    }
    println!("{}  {}", term::dim("active account"), term::accent(&name));
    println!("{}  {}", term::dim("config         "), path.display());
    println!(
        "{}  {}",
        term::dim("api_key        "),
        mask_api_key(key.as_deref())
    );
    println!(
        "{}  {}",
        term::dim("default_model  "),
        term::model_id(&session_model_label(profile.default_model()))
    );
    println!(
        "{}  {}",
        term::dim("claude_haiku   "),
        term::model_id(profile.claude_haiku())
    );
    println!(
        "{}  {}",
        term::dim("claude_sonnet  "),
        term::model_id(profile.claude_sonnet())
    );
    println!(
        "{}  {}",
        term::dim("claude_opus    "),
        term::model_id(profile.claude_opus())
    );
    if let Some(tool) = &profile.default_tool {
        println!(
            "{}  {}",
            term::dim("default_tool   "),
            term::paint(term::tool_color(tool), tool)
        );
    }
    Ok(0)
}
