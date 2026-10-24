//! `anyr pool <stats|providers|free|relay>`: shared-pool health from the public endpoints.
//!
//! Bare `anyr pool` still launches Poolside; the dispatcher only sends here when the first
//! word is one of `SUBCOMMANDS`.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::api::{context, get};
use crate::cmd::view::{print_json, print_table, text};
use crate::parse::ParsedArgs;

pub(crate) const SUBCOMMANDS: &[&str] = &["stats", "providers", "free", "relay"];

pub(crate) fn is_pool_subcommand(parsed: &ParsedArgs) -> bool {
    parsed
        .passthrough
        .first()
        .is_some_and(|w| SUBCOMMANDS.contains(&w.as_str()))
}

pub(crate) fn run_pool(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let [sub] = parsed.passthrough.as_slice() else {
        return Err(crate::cmd::dispatch::hint(
            "Usage: {bin} pool <stats|providers|free|relay> [--json]",
        ));
    };
    let ctx = context(parsed, env);
    // All four endpoints are public: a key is sent when present, never required.
    let fetch = |path: &str| get(&ctx, path, false);
    match sub.as_str() {
        "stats" => {
            let stats = fetch("/v1/pool/stats")?;
            let providers = fetch("/v1/pool/providers")?;
            let free = fetch("/v1/pool/free-coverage")?;
            if ctx.json {
                print_json(&json!({"stats": stats, "providers": providers, "free_coverage": free}));
            } else {
                print!("{}", summary_text(&stats, &providers, &free));
            }
        }
        "providers" => {
            let body = fetch("/v1/pool/providers")?;
            if ctx.json {
                print_json(&body);
            } else if let Some(rows) = provider_rows(&body) {
                print_table(&["provider", "keys", "donors", "recent donors"], &rows);
            } else {
                println!("{}", pool_off_note(&body));
            }
        }
        "free" => {
            let body = fetch("/v1/pool/free-coverage")?;
            if ctx.json {
                print_json(&body);
            } else {
                print!("{}", free_text(&body));
            }
        }
        _ => {
            let body = fetch("/v1/relay/pool/availability")?;
            if ctx.json {
                print_json(&body);
            } else {
                print!("{}", relay_text(&body));
            }
        }
    }
    Ok(0)
}

fn num(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn enabled(v: &Value) -> bool {
    v.get("pool_enabled").and_then(Value::as_bool) == Some(true)
}

fn pool_off_note(body: &Value) -> &'static str {
    if enabled(body) {
        "No donated keys in the pool yet."
    } else {
        "The pool is off in this environment."
    }
}

fn backends(free: &Value) -> Vec<String> {
    free.get("backends")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Table rows, or None when there is nothing to list.
pub(crate) fn provider_rows(body: &Value) -> Option<Vec<Vec<String>>> {
    let list = body.get("providers").and_then(Value::as_array)?;
    if list.is_empty() {
        return None;
    }
    Some(
        list.iter()
            .map(|p| {
                let donors = p
                    .get("donors_masked")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "-".into());
                vec![
                    text(p.get("provider")),
                    num(p, "key_count").to_string(),
                    num(p, "donor_count").to_string(),
                    donors,
                ]
            })
            .collect(),
    )
}

pub(crate) fn summary_text(stats: &Value, providers: &Value, free: &Value) -> String {
    if !enabled(stats) {
        return "The pool is off in this environment.\n".into();
    }
    let keys: u64 = providers
        .get("providers")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|p| num(p, "key_count")).sum())
        .unwrap_or(0);
    let names: Vec<String> = providers
        .get("providers")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("provider").and_then(Value::as_str))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let covered = backends(free);
    let credits = stats
        .get("credits_rewarded_total")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let list = |v: &[String]| {
        if v.is_empty() {
            "-".to_string()
        } else {
            v.join(", ")
        }
    };
    format!(
        "{:<18}{}\n{:<18}{} ({})\n{:<18}{}\n{:<18}{}\n{:<18}{}\n{:<18}{} ({})\n",
        "providers",
        num(stats, "eligible_provider_count"),
        "keys in pool",
        keys,
        list(&names),
        "donated keys",
        num(stats, "active_donors"),
        "requests today",
        num(stats, "requests_today"),
        "credits rewarded",
        crate::http::format_usd(credits),
        "free coverage",
        covered.len(),
        list(&covered),
    )
}

pub(crate) fn free_text(body: &Value) -> String {
    let covered = backends(body);
    if covered.is_empty() {
        return "No donated key can serve free models right now.\n".into();
    }
    let mut out = format!(
        "Backends the pool can serve for free now ({}):\n",
        covered.len()
    );
    for b in covered {
        out += &format!("  {b}\n");
    }
    out
}

pub(crate) fn relay_text(body: &Value) -> String {
    if !enabled(body) {
        return "The relay pool is off in this environment.\n".into();
    }
    let totals = body.get("totals").unwrap_or(&Value::Null);
    let mut out = format!(
        "{:<16}{}\n{:<16}{}\n",
        "online devices",
        num(totals, "online_clients"),
        "models served",
        num(totals, "models_served"),
    );
    let models = body.get("models").and_then(Value::as_array);
    if let Some(models) = models.filter(|m| !m.is_empty()) {
        out.push('\n');
        for m in models {
            out += &format!(
                "  {:<40}{} device(s)\n",
                text(m.get("model")),
                num(m, "device_count")
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_rows_show_counts_and_masked_donors_or_dash() {
        let body = json!({"pool_enabled": true, "providers": [
            {"provider": "openai", "key_count": 7, "donor_count": 4, "donors_masked": ["duy•••t", "a•••b"]},
            {"provider": "groq", "key_count": 1, "donor_count": 1, "donors_masked": []}]});
        let rows = provider_rows(&body).unwrap();
        assert_eq!(rows[0], ["openai", "7", "4", "duy•••t, a•••b"]);
        assert_eq!(rows[1], ["groq", "1", "1", "-"]);
    }

    #[test]
    fn provider_rows_none_when_empty() {
        assert!(provider_rows(&json!({"pool_enabled": true, "providers": []})).is_none());
        assert_eq!(
            pool_off_note(&json!({"pool_enabled": false})),
            "The pool is off in this environment."
        );
        assert_eq!(
            pool_off_note(&json!({"pool_enabled": true})),
            "No donated keys in the pool yet."
        );
    }

    #[test]
    fn summary_sums_keys_and_lists_free_coverage() {
        let stats = json!({"pool_enabled": true, "eligible_provider_count": 2, "active_donors": 9,
            "requests_today": 1234, "credits_rewarded_total": 12.5});
        let providers = json!({"providers": [
            {"provider": "openai", "key_count": 7}, {"provider": "groq", "key_count": 2}]});
        let free = json!({"backends": ["groq", "nvidia"]});
        let s = summary_text(&stats, &providers, &free);
        assert!(s.contains("keys in pool      9 (openai, groq)"), "{s}");
        assert!(s.contains("requests today    1234"), "{s}");
        assert!(s.contains("free coverage     2 (groq, nvidia)"), "{s}");
        assert!(s.contains("$12.50"), "{s}");
    }

    #[test]
    fn summary_reports_disabled_pool() {
        let s = summary_text(&json!({"pool_enabled": false}), &json!({}), &json!({}));
        assert_eq!(s, "The pool is off in this environment.\n");
    }

    #[test]
    fn free_lists_backends_or_says_none() {
        let s = free_text(&json!({"backends": ["groq", "nvidia"]}));
        assert!(
            s.contains("(2)") && s.contains("  groq\n") && s.contains("  nvidia\n"),
            "{s}"
        );
        assert!(free_text(&json!({"backends": []})).contains("No donated key"));
    }

    #[test]
    fn relay_shows_totals_and_per_model_devices() {
        let s = relay_text(&json!({"pool_enabled": true,
            "totals": {"online_clients": 3, "models_served": 2},
            "models": [{"model": "apple/foundation-model", "device_count": 2}]}));
        assert!(s.contains("online devices  3"), "{s}");
        assert!(
            s.contains("apple/foundation-model") && s.contains("2 device(s)"),
            "{s}"
        );
        assert!(relay_text(&json!({"pool_enabled": false})).contains("off"));
    }
}
