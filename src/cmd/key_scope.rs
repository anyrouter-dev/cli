//! Scoping flags for `anyr keys create|update` (spend limit, endpoints, expiry)
//! and the `keys show` view.

use serde_json::{json, Map, Value};

use crate::cmd::dispatch::USAGE;
use crate::http::format_usd;
use crate::parse::{get_string_flag, ParsedArgs};
use crate::term;

const RESETS: &[&str] = &["daily", "weekly", "monthly"];

/// `(name, paths)`. Chat-shaped names are one grant on the server
/// (`CHAT_SHAPED_ENDPOINT_PATHS`), so each expands to the whole group and is
/// never split.
const ENDPOINTS: &[(&str, &[&str])] = &[
    (
        "chat",
        &[
            "/api/v1/chat/completions",
            "/api/v1/messages",
            "/api/v1/responses",
        ],
    ),
    (
        "messages",
        &[
            "/api/v1/chat/completions",
            "/api/v1/messages",
            "/api/v1/responses",
        ],
    ),
    (
        "responses",
        &[
            "/api/v1/chat/completions",
            "/api/v1/messages",
            "/api/v1/responses",
        ],
    ),
    ("embeddings", &["/api/v1/embeddings"]),
    ("moderations", &["/api/v1/moderations"]),
    ("rerank", &["/api/v1/rerank"]),
    ("documents", &["/api/v1/documents/parse"]),
    ("speech", &["/api/v1/audio/speech"]),
    ("decisions", &["/api/v1/systemone"]),
    ("mcp", &["/api/v1/mcp"]),
    ("me", &["/api/v1/me"]),
    ("relay", &["/api/v1/relay"]),
    ("models", &["/api/v1/models"]),
    ("keys", &["/api/v1/keys"]),
    ("credits", &["/api/v1/credits"]),
    ("logs", &["/api/v1/logs"]),
    ("dashboard", &["/api/v1/dashboard"]),
    ("trace", &["/api/v1/trace"]),
];

fn usage(msg: String) -> String {
    format!("{USAGE}{msg}")
}

/// `30d`, `12h`, `45m`, `2w` to seconds.
pub(crate) fn parse_duration(s: &str) -> Result<i64, String> {
    let bad = || {
        usage(format!(
            "--expires must look like 30d, 12h, 45m or 2w, got \"{s}\"."
        ))
    };
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = num.parse().map_err(|_| bad())?;
    let mult = match unit {
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        _ => return Err(bad()),
    };
    if n <= 0 {
        return Err(bad());
    }
    Ok(n * mult)
}

/// Unix seconds to `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn iso_utc(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub(crate) fn endpoint_paths(csv: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for name in csv.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let Some((_, paths)) = ENDPOINTS.iter().find(|(n, _)| *n == name) else {
            let names: Vec<&str> = ENDPOINTS.iter().map(|(n, _)| *n).collect();
            return Err(usage(format!(
                "unknown endpoint \"{name}\"; use a comma list of: {}.",
                names.join(", ")
            )));
        };
        for p in *paths {
            if !out.iter().any(|o| o == p) {
                out.push((*p).to_string());
            }
        }
    }
    if out.is_empty() {
        return Err(usage("--endpoints needs at least one name.".into()));
    }
    Ok(out)
}

/// Request-body fields from `--limit/--reset/--endpoints/--expires` and, for
/// updates, `--disable/--enable`. Only the flags given appear in the body.
pub(crate) fn scope_fields(
    parsed: &ParsedArgs,
    now_secs: i64,
) -> Result<Map<String, Value>, String> {
    let mut body = Map::new();
    if let Some(raw) = get_string_flag(&parsed.flags, "limit") {
        let n: f64 = raw
            .parse()
            .ok()
            .filter(|n: &f64| n.is_finite() && *n >= 0.0)
            .ok_or_else(|| {
                usage(format!(
                    "--limit must be a non-negative number, got \"{raw}\"."
                ))
            })?;
        body.insert("limit".into(), json!(n));
    }
    if let Some(r) = get_string_flag(&parsed.flags, "reset") {
        if !RESETS.contains(&r.as_str()) {
            return Err(usage(format!(
                "--reset must be one of: {}.",
                RESETS.join(", ")
            )));
        }
        body.insert("limit_reset".into(), json!(r));
    }
    if let Some(csv) = get_string_flag(&parsed.flags, "endpoints") {
        body.insert("allowed_endpoints".into(), json!(endpoint_paths(&csv)?));
    }
    if let Some(e) = get_string_flag(&parsed.flags, "expires") {
        body.insert(
            "expires_at".into(),
            json!(iso_utc(now_secs + parse_duration(&e)?)),
        );
    }
    match (parsed.flag_true("disable"), parsed.flag_true("enable")) {
        (true, true) => return Err(usage("pass only one of --disable and --enable.".into())),
        (true, false) => {
            body.insert("disabled".into(), json!(true));
        }
        (false, true) => {
            body.insert("disabled".into(), json!(false));
        }
        _ => {}
    }
    Ok(body)
}

pub(crate) fn render_show(k: &Value) -> String {
    let dim = |s: &str| term::dim(&format!("{s:<10}"));
    let text = |v: Option<&Value>| match v {
        None | Some(Value::Null) => "-".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(o) => o.to_string(),
    };
    let usd = |key: &str| {
        k.get(key)
            .and_then(Value::as_f64)
            .map_or("-".to_string(), format_usd)
    };
    let limit = match k.get("limit").and_then(Value::as_f64) {
        None => "none".to_string(),
        Some(l) => match k.get("limit_reset").and_then(Value::as_str) {
            Some(r) => format!("{} ({r})", format_usd(l)),
            None => format_usd(l),
        },
    };
    let endpoints = match k.get("allowed_endpoints").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a
            .iter()
            .filter_map(Value::as_str)
            .map(|p| p.trim_start_matches("/api/v1/"))
            .collect::<Vec<_>>()
            .join(", "),
        _ => "all".to_string(),
    };
    let status = if k.get("disabled").and_then(Value::as_bool) == Some(true) {
        "disabled"
    } else {
        "active"
    };
    let mut out = String::new();
    for (label, value) in [
        ("name", text(k.get("name"))),
        ("hash", text(k.get("hash"))),
        ("status", status.to_string()),
        ("limit", limit),
        ("spent", usd("real_usage")),
        ("remaining", usd("limit_remaining")),
        ("endpoints", endpoints),
        ("expires", text(k.get("expires_at"))),
    ] {
        out.push_str(&format!("{}  {value}\n", dim(label)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_cli_args;

    fn fields(args: &[&str]) -> Result<Map<String, Value>, String> {
        let mut argv = vec!["keys"];
        argv.extend(args);
        scope_fields(&parse_cli_args(argv).unwrap(), 1_792_806_676)
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30d"), Ok(30 * 86400));
        assert_eq!(parse_duration("12h"), Ok(12 * 3600));
        assert_eq!(parse_duration("2w"), Ok(14 * 86400));
        for bad in ["", "d", "30", "0d", "-1d", "1y", "xd"] {
            assert!(parse_duration(bad).unwrap_err().starts_with(USAGE), "{bad}");
        }
    }

    #[test]
    fn iso_matches_known_instants() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_792_806_676), "2026-10-24T01:51:16Z");
        assert_eq!(iso_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn chat_expands_to_the_whole_chat_grant_and_never_splits() {
        // WHY: the server treats chat/messages/responses as one permission;
        // sending a lone /chat/completions would imply a split that does not exist.
        let want = [
            "/api/v1/chat/completions",
            "/api/v1/messages",
            "/api/v1/responses",
        ];
        assert_eq!(endpoint_paths("chat").unwrap(), want);
        assert_eq!(endpoint_paths("messages").unwrap(), want);
        assert_eq!(
            endpoint_paths("chat,embeddings,responses").unwrap(),
            [want[0], want[1], want[2], "/api/v1/embeddings"]
        );
    }

    #[test]
    fn unknown_endpoint_is_rejected_with_the_valid_names() {
        let e = endpoint_paths("chat,teleport").unwrap_err();
        assert!(
            e.starts_with(USAGE) && e.contains("teleport") && e.contains("embeddings"),
            "{e}"
        );
        assert!(endpoint_paths(" , ").is_err());
    }

    #[test]
    fn fields_only_contain_flags_given() {
        assert!(fields(&[]).unwrap().is_empty());
        let b = fields(&[
            "--limit",
            "25",
            "--reset",
            "monthly",
            "--endpoints",
            "embeddings",
            "--expires",
            "30d",
            "--disable",
        ])
        .unwrap();
        assert_eq!(b["limit"], 25.0);
        assert_eq!(b["limit_reset"], "monthly");
        assert_eq!(b["allowed_endpoints"], json!(["/api/v1/embeddings"]));
        assert_eq!(b["expires_at"], "2026-11-23T01:51:16Z");
        assert_eq!(b["disabled"], true);
    }

    #[test]
    fn bad_limit_reset_and_conflicting_toggles_fail() {
        assert!(fields(&["--limit", "-1"]).is_err());
        assert!(fields(&["--limit", "abc"]).is_err());
        assert!(fields(&["--reset", "hourly"]).is_err());
        assert!(fields(&["--disable", "--enable"]).is_err());
    }

    #[test]
    fn show_lists_limit_spent_endpoints_expiry() {
        let k = json!({"name":"ci","hash":"h1","limit":25.0,"limit_reset":"monthly","real_usage":1.5,
            "limit_remaining":23.5,"allowed_endpoints":["/api/v1/embeddings"],"expires_at":"2026-11-23T01:51:16Z"});
        let out = render_show(&k);
        for want in [
            "$25.00 (monthly)",
            "$1.50",
            "$23.50",
            "embeddings",
            "2026-11-23T01:51:16Z",
            "active",
        ] {
            assert!(out.contains(want), "{want} in\n{out}");
        }
        assert!(render_show(&json!({"name":"x"})).contains("all"));
    }
}
