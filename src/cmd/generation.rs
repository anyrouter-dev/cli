//! `anyr generation <id>` (alias `gen`): final cost, provider and timing of one call.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::api::{context, get};
use crate::cmd::view::{local_time, print_json, text};
use crate::http::format_usd;
use crate::parse::ParsedArgs;

pub(crate) fn run_generation(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let [id] = parsed.passthrough.as_slice() else {
        return Err(crate::cmd::dispatch::hint(
            "Usage: {bin} generation <id> [--json]",
        ));
    };
    let ctx = context(parsed, env);
    let body = get(
        &ctx,
        &format!("/v1/generation/{}", crate::api::encode(id)),
        true,
    )?;
    if ctx.json {
        print_json(&body);
    } else {
        for (label, value) in detail_lines(&body) {
            println!("{label:<11}{value}");
        }
    }
    Ok(0)
}

/// Label and value pairs in display order; missing fields show as `-`.
pub(crate) fn detail_lines(body: &Value) -> Vec<(&'static str, String)> {
    let obj = body.get("data").filter(|d| d.is_object()).unwrap_or(body);
    let field = |k: &str| obj.get(k);
    let num = |k: &str| field(k).and_then(Value::as_f64);
    let dash = |s: Option<String>| s.unwrap_or_else(|| "-".into());
    let status = match (text(field("status")).as_str(), text(field("finish_reason"))) {
        (s, f) if f.is_empty() || f == "-" => s.to_string(),
        (s, f) => format!("{s} ({f})"),
    };
    vec![
        ("id", text(field("id"))),
        ("model", text(field("model"))),
        ("provider", text(field("provider_name"))),
        ("status", status),
        ("tokens in", text(field("tokens_prompt"))),
        ("tokens out", text(field("tokens_completion"))),
        ("cost", dash(num("total_cost").map(format_usd))),
        (
            "latency",
            dash(num("latency_ms").map(|ms| format!("{ms:.0}ms"))),
        ),
        (
            "cache",
            match field("cache_hit").and_then(Value::as_bool) {
                Some(true) => "hit".into(),
                Some(false) => "miss".into(),
                None => "-".into(),
            },
        ),
        (
            "created",
            dash(field("created_at").and_then(Value::as_str).map(local_time)),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn value<'a>(lines: &'a [(&'static str, String)], label: &str) -> &'a str {
        &lines.iter().find(|(l, _)| *l == label).unwrap().1
    }

    #[test]
    fn formats_cost_latency_status_and_cache() {
        let body = json!({"id": "gen_1", "model": "z-ai/glm-4.7", "provider_name": "nvidia",
            "status": "success", "finish_reason": "stop", "tokens_prompt": 12,
            "tokens_completion": 34, "total_cost": 0.0012, "latency_ms": 840.4,
            "cache_hit": true, "created_at": "2026-10-24T01:51:16Z"});
        let l = detail_lines(&body);
        assert_eq!(value(&l, "cost"), "$0.0012");
        assert_eq!(value(&l, "latency"), "840ms");
        assert_eq!(value(&l, "status"), "success (stop)");
        assert_eq!(value(&l, "cache"), "hit");
        assert_eq!(value(&l, "tokens out"), "34");
    }

    #[test]
    fn missing_fields_show_dashes_and_empty_finish_reason_is_dropped() {
        let l = detail_lines(&json!({"id": "g", "status": "error", "finish_reason": ""}));
        assert_eq!(value(&l, "status"), "error");
        assert_eq!(value(&l, "cost"), "-");
        assert_eq!(value(&l, "latency"), "-");
        assert_eq!(value(&l, "cache"), "-");
        assert_eq!(value(&l, "created"), "-");
    }
}
