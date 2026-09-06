use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::config::write_config;
use crate::http::{create_key, delete_key, fetch_keys, is_active_key_row, reveal_key};
use crate::key::{
    load_config_if_present, mask_api_key, no_key_error, resolve_api_key, resolve_base_url,
};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::term;

use crate::cmd::dispatch::{config_path, hint};
use crate::cmd::models::{flag_agent, pick_list, save_agent_key};

pub(crate) fn keys_credential(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<(std::path::PathBuf, crate::config::Config, String, String), String> {
    let path = config_path(parsed, env);
    let cfg = load_config_if_present(&path).ok_or_else(no_key_error)?;
    let name =
        get_string_flag(&parsed.flags, "profile").unwrap_or_else(|| cfg.active_profile.clone());
    let profile = cfg
        .profiles
        .get(&name)
        .ok_or_else(|| format!("Profile \"{name}\" was not found in AnyRouter config."))?;
    let base = resolve_base_url(&parsed.flags, Some(profile));
    let api_key = resolve_api_key(&parsed.flags, env, Some(profile))
        .ok_or_else(|| hint("No stored credential. Run \"{bin} login\" first."))?;
    Ok((path, cfg, base, api_key))
}

pub(crate) fn default_key_name() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "device".into());
    let short = host.split('.').next().unwrap_or("device");
    format!("cli-{short}").chars().take(40).collect()
}

pub(crate) fn run_keys(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let sub = parsed
        .passthrough
        .first()
        .map(String::as_str)
        .unwrap_or("list");
    match sub {
        "list" => {
            let (_path, _cfg, base, api_key) = keys_credential(parsed, env)?;
            let rows = crate::http::keys_newest_first(fetch_keys(&base, &api_key)?);
            if parsed.flag_true("json") {
                let payload: Vec<_> = rows
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "name": r.name,
                            "hash": r.hash,
                            "masked": r.masked,
                            "created_at": r.created_at,
                            "last_used_at": r.last_used_at,
                            "active": r.active,
                            "current": is_active_key_row(&r.masked, Some(&api_key)),
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "[]".into())
                );
                return Ok(0);
            }
            if rows.is_empty() {
                println!(
                    "{}",
                    term::dim(&hint("No API keys. Create one: {bin} keys create"))
                );
                return Ok(0);
            }
            let name_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(4).max(4);
            for r in &rows {
                let marker = if is_active_key_row(&r.masked, Some(&api_key)) {
                    "*"
                } else {
                    " "
                };
                let state = if r.active { "" } else { "  (disabled)" };
                println!(
                    "{marker} {:name_w$}  {}  {}{state}",
                    r.name,
                    r.masked,
                    r.last_used_at.as_deref().unwrap_or("never used")
                );
            }
            println!();
            println!(
                "{}",
                term::dim(&hint("* = key this profile uses · switch: {bin} keys use"))
            );
            Ok(0)
        }
        "create" => {
            let (path, mut cfg, base, cred) = keys_credential(parsed, env)?;
            let name = parsed
                .passthrough
                .get(1)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(default_key_name);
            let key = create_key(&base, &cred, &name)?;
            println!("Created \"{name}\":\n\n  {key}\n\nShown once — store it now.");
            let save = parsed.flag_true("yes")
                || (term::is_interactive()
                    && term::confirm("Use this key for the current profile?"));
            if save {
                let active = cfg.active_profile.clone();
                if let Some(p) = cfg.profiles.get_mut(&active) {
                    p.api_key = Some(key);
                }
                write_config(&cfg, &path)?;
                println!("{}  saved to config.yaml", term::ok("Saved"));
            }
            Ok(0)
        }
        "use" => {
            let (path, mut cfg, base, api_key) = keys_credential(parsed, env)?;
            let rows = crate::http::keys_newest_first(
                fetch_keys(&base, &api_key)?
                    .into_iter()
                    .filter(|r| r.active)
                    .collect(),
            );
            if rows.is_empty() {
                return Err(hint("No active keys. Create one: {bin} keys create"));
            }
            let hash_arg = parsed.passthrough.get(1).cloned();
            let row = if let Some(hash) = hash_arg {
                let matches: Vec<_> = rows
                    .iter()
                    .filter(|r| r.hash == hash || r.hash.starts_with(&hash))
                    .collect();
                match matches.as_slice() {
                    [one] => (*one).clone(),
                    [] => {
                        return Err(hint(&format!(
                            "No key matches \"{hash}\". See: {{bin}} keys list"
                        )))
                    }
                    _ => {
                        return Err(format!(
                            "\"{hash}\" matches {} keys — use a longer hash prefix.",
                            matches.len()
                        ))
                    }
                }
            } else if term::is_interactive() {
                let current = rows
                    .iter()
                    .position(|r| is_active_key_row(&r.masked, Some(&api_key)))
                    .or(Some(0));
                let labels: Vec<String> = rows
                    .iter()
                    .enumerate()
                    .map(|(i, r)| key_pick_label(r, current == Some(i)))
                    .collect();
                let idx = pick_list(
                    "API key",
                    &[
                        "newest first · type to search".into(),
                        format!("current  {}", mask_api_key(Some(&api_key))),
                    ],
                    &labels,
                    current,
                )?;
                rows[idx].clone()
            } else {
                return Err(hint(
                    "Usage: {bin} keys use <hash>   (interactive picker needs a terminal)",
                ));
            };
            let revealed = reveal_key(&base, &api_key, &row.hash)?;
            if let Some(agent) = flag_agent(parsed) {
                return save_agent_key(&path, &agent, &revealed);
            }
            let active = cfg.active_profile.clone();
            if let Some(p) = cfg.profiles.get_mut(&active) {
                p.api_key = Some(revealed);
            }
            write_config(&cfg, &path)?;
            println!(
                "{}  now using {} ({})",
                term::ok("Switched"),
                term::accent(&row.name),
                mask_api_key(cfg.profiles.get(&active).and_then(|p| p.api_key.as_deref()))
            );
            Ok(0)
        }
        "revoke" => {
            let hash = parsed.passthrough.get(1).cloned().ok_or_else(|| {
                hint("Usage: {bin} keys revoke <hash>   (find hashes: {bin} keys list)")
            })?;
            let (_path, _cfg, base, cred) = keys_credential(parsed, env)?;
            let rows = fetch_keys(&base, &cred)?;
            let matches: Vec<_> = rows
                .iter()
                .filter(|r| r.hash == hash || r.hash.starts_with(&hash))
                .collect();
            let row = match matches.as_slice() {
                [one] => *one,
                [] => {
                    return Err(hint(&format!(
                        "No key matches \"{hash}\". See: {{bin}} keys list"
                    )))
                }
                _ => {
                    return Err(format!(
                        "\"{hash}\" matches {} keys — use a longer hash prefix.",
                        matches.len()
                    ))
                }
            };
            if !parsed.flag_true("yes") {
                if !term::is_interactive() {
                    return Err(
                        "Revoking a key is destructive; pass --yes to run non-interactively."
                            .into(),
                    );
                }
                if !term::confirm(&format!("Revoke \"{}\" ({})?", row.name, row.masked)) {
                    return Ok(1);
                }
            }
            delete_key(&base, &cred, &row.hash)?;
            println!(
                "{}  revoked \"{}\" ({})",
                term::ok("Revoked"),
                row.name,
                row.masked
            );
            Ok(0)
        }
        other => Err(format!(
            "Unknown keys subcommand \"{other}\". Try: list | create | use | revoke"
        )),
    }
}

pub(crate) fn resolve_latest_key(base: &str, current: &str) -> String {
    let Ok(rows) = fetch_keys(base, current) else {
        return current.to_string();
    };
    let rows = crate::http::keys_newest_first(rows.into_iter().filter(|r| r.active).collect());
    let Some(latest) = rows.first() else {
        return current.to_string();
    };
    if is_active_key_row(&latest.masked, Some(current)) {
        return current.to_string();
    }
    if !latest.can_reveal {
        // Reveal would 409 for pre-reveal-support rows; the stored key stays.
        return current.to_string();
    }
    reveal_key(base, current, &latest.hash).unwrap_or_else(|_| current.to_string())
}

pub(crate) fn key_pick_label(row: &crate::http::RemoteKey, current: bool) -> String {
    let mut parts = vec![row.name.clone(), row.masked.clone()];
    if let Some(created) = row.created_at.as_deref() {
        parts.push(created.get(..10).unwrap_or(created).to_string());
    }
    if current {
        parts.push("●".into());
    }
    parts.join("  ·  ")
}

pub(crate) fn stored_api_key(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &PathBuf,
) -> Option<String> {
    let existing = load_config_if_present(path);
    let profile = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    resolve_api_key(&parsed.flags, env, profile)
}
