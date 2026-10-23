//! `anyr transactions`: the credit ledger.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::api::{context, get, rows};
use crate::cmd::view::{local_time, parse_limit, print_json, print_table, text};
use crate::http::format_usd;
use crate::parse::{get_string_flag, ParsedArgs};

const MAX_LIMIT: u32 = 100;
const DEFAULT_LIMIT: u32 = 20;

pub(crate) fn run_transactions(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    if !parsed.passthrough.is_empty() {
        return Err(crate::cmd::dispatch::hint(
            "Usage: {bin} transactions [--type <type>] [--limit N] [--json]",
        ));
    }
    let ctx = context(parsed, env);
    let limit = parse_limit(
        get_string_flag(&parsed.flags, "limit").as_deref(),
        MAX_LIMIT,
    )?
    .unwrap_or(DEFAULT_LIMIT);
    let path = format!(
        "/v1/credits/transactions{}",
        query(limit, get_string_flag(&parsed.flags, "type").as_deref())
    );
    let body = get(&ctx, &path, true)?;
    if ctx.json {
        print_json(&body);
        return Ok(0);
    }
    let table = ledger_rows(&body);
    if table.is_empty() {
        eprintln!("(none)");
    } else {
        print_table(&["date", "type", "amount", "balance", "note"], &table);
    }
    Ok(0)
}

fn query(limit: u32, kind: Option<&str>) -> String {
    match kind {
        Some(t) => format!("?limit={limit}&type={}", crate::api::encode(t)),
        None => format!("?limit={limit}"),
    }
}

/// `+$5.00` / `-$0.0042`; zero stays unsigned.
pub(crate) fn signed_usd(n: f64) -> String {
    let body = format_usd(n.abs());
    if n < 0.0 {
        format!("-{body}")
    } else if n > 0.0 {
        format!("+{body}")
    } else {
        body
    }
}

pub(crate) fn ledger_rows(body: &Value) -> Vec<Vec<String>> {
    rows(body)
        .iter()
        .map(|r| {
            let money = |k: &str, signed: bool| {
                r.get(k)
                    .and_then(Value::as_f64)
                    .map_or("-".to_string(), |n| {
                        if signed {
                            signed_usd(n)
                        } else {
                            format_usd(n)
                        }
                    })
            };
            vec![
                r.get("created_at")
                    .and_then(Value::as_str)
                    .map_or("-".into(), local_time),
                text(r.get("type")),
                money("amount", true),
                money("balance_after", false),
                text(r.get("description")),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signed_amounts() {
        assert_eq!(signed_usd(5.0), "+$5.00");
        assert_eq!(signed_usd(-0.0042), "-$0.0042");
        assert_eq!(signed_usd(0.0), "$0.00");
    }

    #[test]
    fn query_defaults_limit_and_filters_type() {
        assert_eq!(query(20, None), "?limit=20");
        assert_eq!(query(100, Some("purchase")), "?limit=100&type=purchase");
    }

    #[test]
    fn rows_cover_positive_negative_and_missing_balance() {
        // WHY: the ledger must show direction at a glance; a missing balance
        // (older rows) must not print as $0.00.
        let body = json!({"data": [
            {"created_at": "2026-10-24T01:51:16Z", "type": "purchase", "amount": 10.0,
             "balance_after": 12.5, "description": "Top up"},
            {"type": "usage", "amount": -0.0042, "description": null}
        ]});
        let r = ledger_rows(&body);
        assert_eq!(r[0][1..], ["purchase", "+$10.00", "$12.50", "Top up"]);
        assert_eq!(r[1][2..], ["-$0.0042", "-", "-"]);
    }
}
