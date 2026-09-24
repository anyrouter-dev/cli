use std::collections::BTreeMap;

use crate::help::command_help;
use crate::key::{load_config_if_present, mask_api_key, no_key_error, resolve_api_key};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::term;

use crate::cmd::account::{run_account_use, run_logout};
use crate::cmd::dispatch::{config_path, hint, shift_passthrough};
use crate::cmd::login::run_login;
use crate::cmd::usage::run_whoami;

pub(crate) fn run_auth(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let sub = parsed
        .passthrough
        .first()
        .map(String::as_str)
        .filter(|s| *s != "-h" && *s != "--help")
        .unwrap_or("");
    if sub.is_empty() {
        print!("{}", command_help("auth").unwrap_or_default());
        return Ok(0);
    }
    let rest = shift_passthrough(parsed);
    match sub {
        "login" | "setup" => run_login(&rest, env),
        "logout" => run_logout(&rest, env),
        "status" => run_whoami(&rest, env),
        "token" => run_auth_token(&rest, env),
        "switch" => run_auth_switch(&rest, env),
        other => Err(format!(
            "unknown command \"{other}\" for \"{} auth\"\n\n{}",
            crate::help::invoked_bin(),
            command_help("auth").unwrap_or_default()
        )),
    }
}

pub(crate) fn run_auth_token(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
    let name = get_string_flag(&parsed.flags, "profile")
        .or_else(|| env.get("ANYROUTER_PROFILE").cloned())
        .unwrap_or_else(|| cfg.active_profile.clone());
    let profile = cfg
        .profiles
        .get(&name)
        .ok_or_else(|| format!("Account \"{name}\" was not found."))?;
    let key = resolve_api_key(&parsed.flags, env, Some(profile)).ok_or_else(no_key_error)?;
    if parsed.flag_true("json") {
        let value = if parsed.flag_true("masked") {
            mask_api_key(Some(&key))
        } else {
            key.clone()
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "account": name,
                "token": value,
            }))
            .unwrap_or_else(|_| "{}".into())
        );
        return Ok(0);
    }
    if parsed.flag_true("masked") {
        println!("{}", mask_api_key(Some(&key)));
    } else {
        println!("{key}");
    }
    Ok(0)
}

pub(crate) fn run_auth_switch(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
    let names: Vec<String> = cfg.profiles.keys().cloned().collect();
    let name = parsed
        .passthrough
        .first()
        .cloned()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if !term::is_interactive() || names.is_empty() {
                return None;
            }
            let current = names.iter().position(|n| n == &cfg.active_profile);
            term::pick("Active account", &names, current)
                .ok()
                .map(|i| names[i].clone())
        })
        .ok_or_else(|| hint("Usage: {bin} auth switch <account>"))?;
    run_account_use(parsed, env, &name)
}
