//! `anyr logs`: recent requests, one request, or requests grouped by session.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::api::{context, get, rows, Ctx};
use crate::cmd::dispatch::{hint, USAGE};
use crate::cmd::view::{local_time, parse_limit, print_json, print_table, text};
use crate::http::format_usd;
use crate::parse::{get_string_flag, ParsedArgs};

const MAX_LIMIT: u32 = 100;
const HEADERS: &[&str] = &[
    "time", "model", "provider", "status", "in", "out", "cost", "latency",
];

pub(crate) fn run_logs(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let ctx = context(parsed, env);
    let limit = parse_limit(
        get_string_flag(&parsed.flags, "limit").as_deref(),
        MAX_LIMIT,
    )?;
    let word = parsed.passthrough.first().map(String::as_str);
    if parsed.passthrough.len() > 1 {
        return Err(hint("Usage: {bin} logs [<id>|sessions] [--limit N] [--status ok|error] [--model <id>] [--json]"));
    }
    match word {
        Some("sessions") => sessions(&ctx, limit),
        Some(id) => detail(&ctx, id),
        None => {
            let status = status_param(get_string_flag(&parsed.flags, "status").as_deref())?;
            let path = format!(
                "/v1/logs{}",
                list_query(limit, get_string_flag(&parsed.flags, "model"), status)
            );
            let body = get(&ctx, &path, true)?;
            if ctx.json {
                print_json(&body);
            } else {
                print_logs(&body);
            }
            Ok(0)
        }
    }
}

/// `ok` is the CLI word for the server's `success`.
pub(crate) fn status_param(raw: Option<&str>) -> Result<Option<&'static str>, String> {
    match raw {
        None => Ok(None),
        Some("ok") => Ok(Some("success")),
        Some("error") => Ok(Some("error")),
        Some(other) => Err(format!(
            "{USAGE}--status must be ok or error, got \"{other}\"."
        )),
    }
}

pub(crate) fn list_query(
    limit: Option<u32>,
    model: Option<String>,
    status: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if let Some(n) = limit {
        parts.push(format!("limit={n}"));
    }
    if let Some(m) = model {
        parts.push(format!("model={}", crate::api::encode(&m)));
    }
    if let Some(s) = status {
        parts.push(format!("status={s}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

fn sessions(ctx: &Ctx, limit: Option<u32>) -> Result<i32, String> {
    let q = limit.map(|n| format!("?limit={n}")).unwrap_or_default();
    let body = get(ctx, &format!("/v1/logs/sessions{q}"), true)?;
    if ctx.json {
        print_json(&body);
        return Ok(0);
    }
    let table: Vec<Vec<String>> = rows(&body)
        .iter()
        .map(|r| {
            let cost = r
                .get("cost_usd")
                .and_then(Value::as_f64)
                .map(format_usd)
                .unwrap_or_else(|| "-".into());
            vec![
                text(r.get("session_id")),
                text(r.get("requests")),
                text(r.get("errors")),
                cost,
                r.get("last_seen")
                    .and_then(Value::as_str)
                    .map(local_time)
                    .unwrap_or_else(|| "-".into()),
            ]
        })
        .collect();
    if table.is_empty() {
        eprintln!("(none)");
    } else {
        print_table(
            &["session", "requests", "errors", "cost", "last seen"],
            &table,
        );
    }
    Ok(0)
}

fn detail(ctx: &Ctx, id: &str) -> Result<i32, String> {
    let body = get(ctx, &format!("/v1/logs/{}", crate::api::encode(id)), true)?;
    crate::api::show(ctx, body, &[])
}

/// One table row per request, newest first as the API returns them.
pub(crate) fn log_rows(body: &Value) -> Vec<Vec<String>> {
    rows(body)
        .iter()
        .map(|r| {
            let status = match (
                text(r.get("status")).as_str(),
                r.get("error_code").and_then(Value::as_str),
            ) {
                ("success", _) => "ok".to_string(),
                (s, Some(code)) => format!("{s} ({code})"),
                (s, None) => s.to_string(),
            };
            let latency = r
                .get("latency_ms")
                .and_then(Value::as_f64)
                .map(|ms| format!("{ms:.0}ms"))
                .unwrap_or_else(|| "-".into());
            let cost = r
                .get("cost_usd")
                .and_then(Value::as_f64)
                .map(format_usd)
                .unwrap_or_else(|| "-".into());
            vec![
                r.get("created_at")
                    .and_then(Value::as_str)
                    .map(local_time)
                    .unwrap_or_else(|| "-".into()),
                text(r.get("model")),
                text(r.get("provider_id")),
                status,
                text(r.get("input_tokens")),
                text(r.get("output_tokens")),
                cost,
                latency,
            ]
        })
        .collect()
}

fn print_logs(body: &Value) {
    let table = log_rows(body);
    if table.is_empty() {
        eprintln!("(none)");
    } else {
        print_table(HEADERS, &table);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_maps_ok_to_success() {
        assert_eq!(status_param(Some("ok")), Ok(Some("success")));
        assert_eq!(status_param(Some("error")), Ok(Some("error")));
        assert_eq!(status_param(None), Ok(None));
        assert!(status_param(Some("rejected")).is_err());
    }

    #[test]
    fn query_carries_limit_model_status() {
        assert_eq!(list_query(None, None, None), "");
        assert_eq!(
            list_query(Some(50), Some("z-ai/glm-4.7".into()), Some("error")),
            "?limit=50&model=z-ai/glm-4.7&status=error"
        );
    }

    #[test]
    fn rows_show_error_code_inline_and_missing_fields() {
        let body = json!({"data": [
            {"created_at": "2026-10-24T01:51:16Z", "model": "a/b", "provider_id": "nvidia",
             "status": "success", "input_tokens": 10, "output_tokens": 5,
             "cost_usd": 0.0012, "latency_ms": 840.4},
            {"model": "c/d", "status": "error", "error_code": "upstream_429"}
        ]});
        let r = log_rows(&body);
        assert_eq!(r[0][3], "ok");
        assert_eq!(r[0][4..], ["10", "5", "$0.0012", "840ms"]);
        assert_eq!(r[1][3], "error (upstream_429)");
        assert_eq!(r[1][0], "-");
        assert_eq!(r[1][6], "-");
    }
}
