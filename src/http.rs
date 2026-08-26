#[cfg(feature = "native")]
use crate::VERSION;

#[cfg(feature = "native")]
fn agent() -> ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT
        .get_or_init(|| {
            ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(30))
                .user_agent(&format!("anyr-cli/{VERSION}"))
                .build()
        })
        .clone()
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

#[cfg(feature = "native")]
fn with_auth(mut req: ureq::Request, api_key: Option<&str>) -> ureq::Request {
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    req
}

#[cfg(feature = "native")]
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

#[cfg(feature = "native")]
pub fn http_get(url: &str, api_key: Option<&str>) -> Result<(u16, String), String> {
    into_status_body(with_auth(agent().get(url), api_key).call())
}

#[cfg(feature = "native")]
pub fn http_post(
    url: &str,
    api_key: Option<&str>,
    json_body: Option<&str>,
) -> Result<(u16, String), String> {
    let req = with_auth(agent().post(url), api_key).set("Content-Type", "application/json");
    into_status_body(req.send_string(json_body.unwrap_or("{}")))
}

#[cfg(feature = "native")]
pub fn http_delete(url: &str, api_key: Option<&str>) -> Result<(u16, String), String> {
    into_status_body(with_auth(agent().delete(url), api_key).call())
}

#[cfg(feature = "native")]
pub fn http_patch(
    url: &str,
    api_key: Option<&str>,
    json_body: Option<&str>,
) -> Result<(u16, String), String> {
    let req = with_auth(agent().patch(url), api_key).set("Content-Type", "application/json");
    into_status_body(req.send_string(json_body.unwrap_or("{}")))
}

#[cfg(not(feature = "native"))]
fn no_network() -> Result<(u16, String), String> {
    Err("network is not available in the browser demo".into())
}

#[cfg(not(feature = "native"))]
pub fn http_get(_url: &str, _api_key: Option<&str>) -> Result<(u16, String), String> {
    no_network()
}

#[cfg(not(feature = "native"))]
pub fn http_post(
    _url: &str,
    _api_key: Option<&str>,
    _json_body: Option<&str>,
) -> Result<(u16, String), String> {
    no_network()
}

#[cfg(not(feature = "native"))]
pub fn http_delete(_url: &str, _api_key: Option<&str>) -> Result<(u16, String), String> {
    no_network()
}

#[cfg(not(feature = "native"))]
pub fn http_patch(
    _url: &str,
    _api_key: Option<&str>,
    _json_body: Option<&str>,
) -> Result<(u16, String), String> {
    no_network()
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
    fetch_models_sorted(base_url, api_key, "usage")
}

pub fn fetch_models_sorted(
    base_url: &str,
    api_key: Option<&str>,
    sort: &str,
) -> Result<Vec<CatalogModel>, String> {
    let path = format!("/v1/models?privacy=0&sort={sort}");
    let url = join_api(base_url, &path);
    let (status, body) = http_get(&url, api_key)?;
    if !(200..300).contains(&status) {
        return Err(format!("Could not fetch models (HTTP {status})."));
    }
    parse_models_body(&body)
}

/// First catalog id from a `sort=usage` list (7-day request volume).
pub fn most_used_model_id(models: &[CatalogModel]) -> Option<String> {
    models.iter().find_map(|m| {
        let id = crate::spawn::catalog_model_id(&m.id);
        if id.is_empty() || crate::spawn::is_auto_model(&id) {
            None
        } else {
            Some(id)
        }
    })
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

/// Identity from GET `/v1/me` (accepts `sk-ar-` inference keys).
#[derive(Debug, Clone, Default)]
pub struct MeInfo {
    pub email: Option<String>,
    pub name: Option<String>,
    pub username: Option<String>,
    pub balance: Option<f64>,
}

impl MeInfo {
    /// `username · email` style label, falling back through name/email/username.
    pub fn display_label(&self) -> String {
        let handle = self
            .username
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(self.name.as_deref().filter(|s| !s.is_empty()))
            .or(self.email.as_deref().filter(|s| !s.is_empty()));
        match (handle, self.email.as_deref()) {
            (Some(h), Some(e)) if !e.eq_ignore_ascii_case(h) => format!("{h} · {e}"),
            (Some(h), _) => h.to_string(),
            (None, _) => "—".into(),
        }
    }
}

pub fn parse_me(body: &str) -> Result<MeInfo, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("Invalid /me response: {e}"))?;
    Ok(MeInfo {
        email: value
            .get("email")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        username: value
            .get("username")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        balance: value.get("balance").and_then(|v| v.as_f64()),
    })
}

pub fn fetch_me(base_url: &str, api_key: &str) -> Result<MeInfo, String> {
    let url = join_api(base_url, "/v1/me");
    let (status, body) = http_get(&url, Some(api_key))?;
    if !(200..300).contains(&status) {
        return Err(format!("Could not fetch account info (HTTP {status})."));
    }
    parse_me(&body)
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
        let bin = crate::help::invoked_bin();
        return Err(format!(
            "Not authorized to list keys. Your API key needs Key Management permission — \
run `{bin} auth login` (creates a CLI key with full access) or enable Key Management on the key in the dashboard."
        ));
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

/// Is this listed key the one currently in use? The list endpoint returns a
/// mask, never the secret — and masks come in two shapes:
///
///   * label-style `sk-ar-v1-abcd…` — everything before the separator is a
///     literal head of the key; compare with starts_with (≥12 chars so a
///     generic prefix like `sk-ar-v1-` can't match every row).
///   * the server's getKeyPrefix() shape `sk-ar-v1-...x9K2` — head + ASCII
///     dots + the key's LAST 4 characters. The head alone is only 9 chars
///     (below the threshold), but the tail is real key material: match with
///     starts_with(head) && ends_with(tail).
///
/// Treating the whole `head...tail` mask as one prefix never matches anything
/// (the middle chars differ from the real key), which made login decide its
/// own stored key "wasn't the newest" and re-reveal/rotate on every run.
pub fn is_active_key_row(masked: &str, api_key: Option<&str>) -> bool {
    let key = api_key.unwrap_or("");
    if key.is_empty() || masked.is_empty() {
        return false;
    }
    if let Some((head, tail)) = masked.split_once("...") {
        // A clean head+tail mask (no further separators inside the tail).
        if !tail.is_empty() && !tail.contains(['…', '.', '*']) {
            return head.len() + tail.len() >= 12 && key.starts_with(head) && key.ends_with(tail);
        }
    }
    // Label-style: everything before the first separator is the head.
    let head = masked.split(['…', '.', '*']).next().unwrap_or("");
    head.len() >= 12 && key.starts_with(head)
}

/// Newest `created_at` first. Missing timestamps sort last.
pub fn keys_newest_first(mut keys: Vec<RemoteKey>) -> Vec<RemoteKey> {
    keys.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.hash.cmp(&a.hash))
    });
    keys
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
    fn parse_me_reads_identity_and_balance() {
        let me = parse_me(
            r#"{"id":"user_1","email":"a@b.co","name":"duyet","image_url":"https://x/y.png","username":"duyet","balance":129.22}"#,
        )
        .unwrap();
        assert_eq!(me.email.as_deref(), Some("a@b.co"));
        assert_eq!(me.username.as_deref(), Some("duyet"));
        assert_eq!(me.balance, Some(129.22));
        assert_eq!(me.display_label(), "duyet · a@b.co");
    }

    #[test]
    fn parse_models_reads_context_and_most_used_is_first() {
        let models = parse_models_body(
            r#"{"data":[
                {"id":"stealth/ox-alpha","context_length":1000000},
                {"id":"openai/gpt-5.4-mini","context_length":128000}
            ]}"#,
        )
        .unwrap();
        assert_eq!(models[0].id, "stealth/ox-alpha");
        assert_eq!(models[0].context_length, Some(1_000_000));
        assert_eq!(
            most_used_model_id(&models).as_deref(),
            Some("stealth/ox-alpha")
        );
    }

    #[test]
    fn keys_newest_first_orders_by_created_at() {
        let keys = vec![
            RemoteKey {
                name: "old".into(),
                hash: "h1".into(),
                masked: "sk-ar-v1-aaaa".into(),
                created_at: Some("2026-01-01T00:00:00Z".into()),
                last_used_at: None,
                active: true,
                can_reveal: true,
            },
            RemoteKey {
                name: "new".into(),
                hash: "h2".into(),
                masked: "sk-ar-v1-bbbb".into(),
                created_at: Some("2026-08-24T12:00:00Z".into()),
                last_used_at: None,
                active: true,
                can_reveal: true,
            },
            RemoteKey {
                name: "undated".into(),
                hash: "h0".into(),
                masked: "sk-ar-v1-cccc".into(),
                created_at: None,
                last_used_at: None,
                active: true,
                can_reveal: true,
            },
        ];
        let sorted = keys_newest_first(keys);
        assert_eq!(
            sorted.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(),
            vec!["new", "old", "undated"]
        );
    }

    #[test]
    fn me_display_label_falls_back_gracefully() {
        let mut me = MeInfo::default();
        assert_eq!(me.display_label(), "—");
        me.email = Some("solo@b.co".into());
        assert_eq!(me.display_label(), "solo@b.co");
        me.name = Some("Solo".into());
        assert_eq!(me.display_label(), "Solo · solo@b.co");
        // Email used as handle should not repeat.
        me.username = Some("solo@b.co".into());
        assert_eq!(me.display_label(), "solo@b.co");
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

    #[test]
    fn is_active_key_row_matches_server_getkeyprefix_mask() {
        // Server getKeyPrefix(): first 9 chars + "..." + last 4. The tail is
        // NOT part of the key's head — matching on the whole mask never
        // succeeds and made every login think its stored key was a different
        // one (relogin / re-reveal loop).
        let mask = "sk-ar-v1-...x9K2";
        assert!(is_active_key_row(
            mask,
            Some("sk-ar-v1-Ab3dF7hJ9kLmNpQrStUvWxYz0123456789x9K2")
        ));
        // A different key with the same head must NOT match.
        assert!(!is_active_key_row(
            mask,
            Some("sk-ar-v1-DifferentTail00000000000000000000abcd")
        ));
        // A longer-head ASCII-dot mask matches on head AND literal tail.
        let long = getKeyPrefixStyleMask("sk-ar-v1-abcd", "wxyz");
        assert!(is_active_key_row(
            &long,
            Some("sk-ar-v1-abcd-middle-secret-wxyz")
        ));
        assert!(!is_active_key_row(&long, Some("sk-ar-v1-abcd-other-tail")));
    }

    /// Helper mirroring the server's getKeyPrefix() shape for tests.
    fn getKeyPrefixStyleMask(head: &str, tail: &str) -> String {
        format!("{head}...{tail}")
    }
}
