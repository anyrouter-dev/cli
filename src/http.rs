use std::time::Duration;

use crate::VERSION;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .user_agent(&format!("anyr-cli/{VERSION}"))
        .build()
}

/// Join a CLI profile `base_url` (`https://anyrouter.dev/api`, no `/v1`) with
/// an SDK path. JS `createClient` appends `/v1`, so credits is
/// `https://anyrouter.dev/api/v1/credits`.
pub fn join_api(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{base}{path}")
}

pub fn http_get(url: &str, api_key: Option<&str>) -> Result<(u16, String), String> {
    let mut req = agent().get(url);
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    match req.call() {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.into_string().unwrap_or_default();
            Ok((status, body))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Ok((code, body))
        }
        Err(err) => Err(format!("Could not reach AnyRouter: {err}")),
    }
}

/// GET `{base}/v1/credits`. 401 = rejected. 403 falls back to GET `{base}/v1/models`.
pub fn validate_key(base_url: &str, api_key: &str) -> Result<(), String> {
    let credits_url = join_api(base_url, "/v1/credits");
    let (status, _body) = http_get(&credits_url, Some(api_key))?;
    match status {
        200..=299 => Ok(()),
        401 => Err("AnyRouter key was rejected. Check the key in Dashboard -> Keys.".into()),
        403 => {
            let models_url = join_api(base_url, "/v1/models");
            let (probe, _) = http_get(&models_url, Some(api_key))?;
            if (200..300).contains(&probe) {
                Ok(())
            } else if probe == 401 {
                Err("AnyRouter key was rejected. Check the key in Dashboard -> Keys.".into())
            } else {
                Err("HTTP 403 from GET /credits".into())
            }
        }
        other => Err(format!("Could not validate key (HTTP {other}).")),
    }
}

#[derive(Debug, Clone)]
pub struct CatalogModel {
    pub id: String,
    pub name: Option<String>,
    pub owned_by: Option<String>,
    pub context_length: Option<i64>,
}

pub fn fetch_models(base_url: &str, api_key: Option<&str>) -> Result<Vec<CatalogModel>, String> {
    let url = join_api(base_url, "/v1/models?privacy=0");
    let (status, body) = http_get(&url, api_key)?;
    if !(200..300).contains(&status) {
        return Err(format!("Could not fetch models (HTTP {status})."));
    }
    parse_models_body(&body)
}

fn parse_models_body(body: &str) -> Result<Vec<CatalogModel>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("Invalid models response: {e}"))?;
    let list = value
        .as_array()
        .cloned()
        .or_else(|| value.get("data").and_then(|v| v.as_array()).cloned())
        .or_else(|| value.get("models").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default();
    Ok(list
        .into_iter()
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?.to_string();
            Some(CatalogModel {
                id,
                name: item.get("name").and_then(|v| v.as_str()).map(str::to_string),
                owned_by: item
                    .get("owned_by")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                context_length: item.get("context_length").and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_u64().map(|n| n as i64))
                        .or_else(|| v.as_f64().map(|n| n as i64))
                }),
            })
        })
        .collect())
}

pub fn format_models_list(
    models: &[CatalogModel],
    pinned_ids: &[String],
    pinned_preset: Option<&str>,
    json: bool,
) -> (String, String) {
    let auto_id = pinned_ids.first().cloned();
    if json {
        let payload = serde_json::json!({
            "auto": auto_id,
            "preset": pinned_preset,
            "models": models.iter().map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "name": m.name,
                    "owned_by": m.owned_by,
                    "context_length": m.context_length,
                    "pinned": pinned_ids.iter().any(|id| id == &m.id),
                })
            }).collect::<Vec<_>>(),
        });
        return (
            format!(
                "{}\n",
                serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
            ),
            String::new(),
        );
    }
    let mut out_lines = Vec::new();
    if let Some(auto) = &auto_id {
        out_lines.push(format!(
            "auto -> {auto} ({})",
            pinned_preset.unwrap_or("preset")
        ));
    }
    for model in models {
        out_lines.push(format!("  {}", model.id));
    }
    (format!("{}\n", out_lines.join("\n")), String::new())
}

pub fn fetch_credits(base_url: &str, api_key: &str) -> Result<serde_json::Value, String> {
    let url = join_api(base_url, "/v1/credits");
    let (status, body) = http_get(&url, Some(api_key))?;
    if !(200..300).contains(&status) {
        return Err(format!("Could not fetch usage (HTTP {status})."));
    }
    serde_json::from_str(&body).map_err(|e| format!("Invalid credits response: {e}"))
}

pub fn format_usd(n: f64) -> String {
    if !n.is_finite() {
        return "$?".into();
    }
    let abs = n.abs();
    let digits = if abs > 0.0 && abs < 0.01 { 4 } else { 2 };
    format!("${n:.digits$}")
}

pub fn format_usage_report(credits: &serde_json::Value, json: bool) -> String {
    if json {
        return format!(
            "{}\n",
            serde_json::to_string_pretty(credits).unwrap_or_else(|_| "{}".into())
        );
    }
    let mut lines = Vec::new();
    if let Some(balance) = credits.get("balance").and_then(|v| v.as_f64()) {
        lines.push(format!("Credits remaining  {}", format_usd(balance)));
    } else {
        lines.push("Credits remaining  (unknown)".into());
    }
    format!("{}\n", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_BASE_URL;

    #[test]
    fn join_api_appends_v1_like_js_create_client() {
        assert_eq!(
            join_api(DEFAULT_BASE_URL, "/v1/credits"),
            "https://anyrouter.dev/api/v1/credits"
        );
        assert_eq!(
            join_api(DEFAULT_BASE_URL, "/v1/models"),
            "https://anyrouter.dev/api/v1/models"
        );
        assert_eq!(
            join_api("http://127.0.0.1:9/api", "/v1/credits"),
            "http://127.0.0.1:9/api/v1/credits"
        );
    }
}
