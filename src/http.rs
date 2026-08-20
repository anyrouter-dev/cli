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

fn with_auth(mut req: ureq::Request, api_key: Option<&str>) -> ureq::Request {
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    req
}

fn into_status_body(result: Result<ureq::Response, ureq::Error>) -> Result<(u16, String), String> {
    match result {
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

pub fn http_get(url: &str, api_key: Option<&str>) -> Result<(u16, String), String> {
    into_status_body(with_auth(agent().get(url), api_key).call())
}

pub fn http_post(
    url: &str,
    api_key: Option<&str>,
    json_body: Option<&str>,
) -> Result<(u16, String), String> {
    let req = with_auth(agent().post(url), api_key).set("Content-Type", "application/json");
    into_status_body(req.send_string(json_body.unwrap_or("{}")))
}

pub fn http_delete(url: &str, api_key: Option<&str>) -> Result<(u16, String), String> {
    into_status_body(with_auth(agent().delete(url), api_key).call())
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
                name: item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
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
        lines.push(format!(
            "{}  {}",
            crate::term::dim("Credits remaining"),
            crate::term::ok(&format_usd(balance))
        ));
    } else {
        lines.push(format!(
            "{}  {}",
            crate::term::dim("Credits remaining"),
            crate::term::warn("(unknown)")
        ));
    }
    if let Some(used) = credits
        .get("total_usage")
        .or_else(|| credits.get("lifetime_usage"))
        .or_else(|| credits.get("used"))
        .and_then(|v| v.as_f64())
    {
        lines.push(format!(
            "{}  {}",
            crate::term::dim("Lifetime used     "),
            format_usd(used)
        ));
    }
    format!("{}\n", lines.join("\n"))
}

#[derive(Debug, Clone)]
pub struct RemoteKey {
    pub name: String,
    pub hash: String,
    pub masked: String,
    pub created_at: Option<String>,
    pub last_used_at: Option<String>,
    pub active: bool,
    pub can_reveal: bool,
}

pub fn parse_key_list(body: &str) -> Result<Vec<RemoteKey>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("Invalid keys response: {e}"))?;
    let list = value
        .as_array()
        .cloned()
        .or_else(|| value.get("data").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default();
    Ok(list
        .into_iter()
        .filter_map(|item| {
            let hash = item.get("hash")?.as_str()?.to_string();
            let label = item
                .get("label")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("key_prefix").and_then(|v| v.as_str()))
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if hash.len() >= 8 {
                        format!("…{}", &hash[hash.len().saturating_sub(6)..])
                    } else {
                        "—".into()
                    }
                });
            Some(RemoteKey {
                name: item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("(unnamed)")
                    .to_string(),
                hash,
                masked: label,
                created_at: item
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                last_used_at: item
                    .get("last_used_at")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                active: item.get("disabled").and_then(|v| v.as_bool()) != Some(true),
                can_reveal: item.get("can_reveal").and_then(|v| v.as_bool()) == Some(true),
            })
        })
        .collect())
}

pub fn fetch_keys(base_url: &str, credential: &str) -> Result<Vec<RemoteKey>, String> {
    let url = join_api(base_url, "/v1/keys");
    let (status, body) = http_get(&url, Some(credential))?;
    if status == 401 || status == 403 {
        return Err(
            "Not authorized to list keys — this needs a management key (ak_…). \
Log in again (device/browser login stores one) or pass --management-key."
                .into(),
        );
    }
    if !(200..300).contains(&status) {
        return Err(format!("Could not list keys (HTTP {status})."));
    }
    parse_key_list(&body)
}

pub fn create_key(base_url: &str, credential: &str, name: &str) -> Result<String, String> {
    let url = join_api(base_url, "/v1/keys");
    let body = serde_json::json!({ "name": name }).to_string();
    let (status, resp) = http_post(&url, Some(credential), Some(&body))?;
    if !(200..300).contains(&status) {
        return Err(format!("Could not create key (HTTP {status})."));
    }
    let value: serde_json::Value =
        serde_json::from_str(&resp).map_err(|e| format!("Invalid create-key response: {e}"))?;
    value
        .get("key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Create-key response did not include a key.".into())
}

pub fn reveal_key(base_url: &str, credential: &str, hash: &str) -> Result<String, String> {
    let url = join_api(base_url, &format!("/v1/keys/{hash}/reveal"));
    let (status, resp) = http_post(&url, Some(credential), None)?;
    if status == 409 {
        return Err(
            "That key cannot be revealed (created before reveal support). Create a fresh one: anyr keys create"
                .into(),
        );
    }
    if !(200..300).contains(&status) {
        return Err(format!("Could not fetch the key (HTTP {status})."));
    }
    let value: serde_json::Value =
        serde_json::from_str(&resp).map_err(|e| format!("Invalid reveal response: {e}"))?;
    value
        .get("key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Reveal response did not include a key.".into())
}

pub fn delete_key(base_url: &str, credential: &str, hash: &str) -> Result<(), String> {
    let url = join_api(base_url, &format!("/v1/keys/{hash}"));
    let (status, _body) = http_delete(&url, Some(credential))?;
    if !(200..300).contains(&status) {
        return Err(format!("Could not revoke key (HTTP {status})."));
    }
    Ok(())
}

pub fn is_active_key_row(masked: &str, api_key: Option<&str>) -> bool {
    let key = api_key.unwrap_or("");
    let prefix = masked.split(['…', '*']).next().unwrap_or("");
    !key.is_empty() && prefix.len() >= 12 && key.starts_with(prefix)
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

    #[test]
    fn parse_key_list_accepts_data_wrapper() {
        let keys = parse_key_list(
            r#"{"data":[{"hash":"abc123def456","name":"laptop","key_prefix":"sk-ar-v1-abcd","can_reveal":true}]}"#,
        )
        .unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].name, "laptop");
        assert_eq!(keys[0].masked, "sk-ar-v1-abcd");
        assert!(keys[0].can_reveal);
    }

    #[test]
    fn is_active_key_row_matches_prefix_only() {
        assert!(is_active_key_row(
            "sk-ar-v1-abcd…zzzz",
            Some("sk-ar-v1-abcd-secret")
        ));
        assert!(!is_active_key_row(
            "sk-ar-v1-abcd…zzzz",
            Some("sk-ar-v1-zzzz")
        ));
        assert!(!is_active_key_row("short", Some("sk-ar-v1-abcd-secret")));
    }
}
