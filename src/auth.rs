//! Login acquisition: --key / env, paste, device-code (RFC 8628-style
//! `/v1/auth/cli/device/*`). Browser GUI opens the verification URL.

use std::collections::BTreeMap;
#[cfg(feature = "native")]
use std::io::{self, Write};
#[cfg(feature = "native")]
use std::process::Command;
#[cfg(feature = "native")]
use std::thread;
#[cfg(feature = "native")]
use std::time::Duration;

#[cfg(feature = "native")]
use crate::http::{http_post, join_api};
use crate::key::no_key_error;
use crate::parse::{get_string_flag, FlagValue};
use crate::term;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceToken {
    pub api_key: String,
    pub management_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    Pending,
    SlowDown,
    Denied,
    Expired,
    Failed(String),
    Ready(DeviceToken),
}

pub fn parse_device_start(body: &str) -> Result<DeviceStart, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("Invalid device-start response: {e}"))?;
    let device_code = value
        .get("device_code")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Device login response missing device_code.".to_string())?
        .to_string();
    let user_code = value
        .get("user_code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let verification_uri = value
        .get("verification_uri")
        .or_else(|| value.get("verification_uri_complete"))
        .and_then(|v| v.as_str())
        .unwrap_or("https://anyrouter.dev/cli/device")
        .to_string();
    let interval = value.get("interval").and_then(|v| v.as_u64()).unwrap_or(5);
    let expires_in = value
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(600);
    Ok(DeviceStart {
        device_code,
        user_code,
        verification_uri,
        interval,
        expires_in,
    })
}

fn error_code(value: &serde_json::Value) -> String {
    if let Some(s) = value.get("error").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(s) = value
        .get("error")
        .and_then(|v| v.get("code"))
        .and_then(|v| v.as_str())
    {
        return s.to_string();
    }
    String::new()
}

pub fn parse_device_token(status: u16, body: &str) -> DevicePoll {
    if (200..300).contains(&status) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
            return DevicePoll::Failed("Device login response was not JSON.".into());
        };
        let Some(api_key) = value
            .get("key")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            return DevicePoll::Failed("Device login response did not include an API key.".into());
        };
        let management_key = value
            .get("management_key")
            .and_then(|v| v.get("secret"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        return DevicePoll::Ready(DeviceToken {
            api_key,
            management_key,
        });
    }
    let value: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    match error_code(&value).as_str() {
        "authorization_pending" => DevicePoll::Pending,
        "slow_down" => DevicePoll::SlowDown,
        "access_denied" => DevicePoll::Denied,
        "expired_token" => DevicePoll::Expired,
        other => DevicePoll::Failed(format!(
            "Device login failed (HTTP {status}{}).",
            if other.is_empty() {
                String::new()
            } else {
                format!(" {other}")
            }
        )),
    }
}

pub fn browser_likely_available(env: &BTreeMap<String, String>) -> bool {
    if env.contains_key("SSH_CONNECTION") || env.contains_key("SSH_TTY") {
        return false;
    }
    if env.get("CI").map(|s| !s.is_empty()).unwrap_or(false) {
        return false;
    }
    if cfg!(target_os = "linux") && env.get("DISPLAY").map(|s| s.is_empty()).unwrap_or(true) {
        // Wayland-only desktops still count as a GUI.
        if env
            .get("WAYLAND_DISPLAY")
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        return false;
    }
    true
}

#[cfg(not(feature = "native"))]
pub fn open_url(_url: &str) -> bool {
    false
}

#[cfg(feature = "native")]
pub fn open_url(url: &str) -> bool {
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", "", url]).status()
    } else {
        Command::new("xdg-open").arg(url).status()
    };
    status.map(|s| s.success()).unwrap_or(false)
}

#[cfg(feature = "native")]
fn print_device_block(start: &DeviceStart) {
    let mins = (start.expires_in / 60).max(1);
    eprintln!();
    eprintln!("  {}", term::bold("Device login"));
    eprintln!("  {}", term::divider(12));
    eprintln!("  Open this URL in your browser (you must be signed in to AnyRouter):");
    eprintln!();
    eprintln!("    {}", term::accent(&start.verification_uri));
    eprintln!();
    eprintln!("  Then enter this code:");
    eprintln!();
    eprintln!("    {}", term::bold(&start.user_code));
    eprintln!();
    eprintln!(
        "  {}",
        term::dim(&format!(
            "Code expires in {mins} minutes. Waiting for approval…"
        ))
    );
    eprintln!();
    let _ = io::stderr().flush();
}

#[cfg(not(feature = "native"))]
pub fn start_device_flow(_base_url: &str, _tool: Option<&str>) -> Result<DeviceStart, String> {
    Err("device login is not available in the browser demo".into())
}

#[cfg(feature = "native")]
pub fn start_device_flow(base_url: &str, tool: Option<&str>) -> Result<DeviceStart, String> {
    let url = join_api(base_url, "/v1/auth/cli/device/start");
    let body = match tool {
        Some(t) if !t.is_empty() => serde_json::json!({ "tool": t }).to_string(),
        _ => "{}".into(),
    };
    let (status, resp) = http_post(&url, None, Some(&body))?;
    if !(200..300).contains(&status) {
        return Err(format!("Failed to start device login (HTTP {status})."));
    }
    parse_device_start(&resp)
}

#[cfg(not(feature = "native"))]
pub fn poll_device_token(_base_url: &str, _start: &DeviceStart) -> Result<DeviceToken, String> {
    Err("device login is not available in the browser demo".into())
}

#[cfg(feature = "native")]
pub fn poll_device_token(base_url: &str, start: &DeviceStart) -> Result<DeviceToken, String> {
    let url = join_api(base_url, "/v1/auth/cli/device/token");
    let payload = serde_json::json!({ "device_code": start.device_code }).to_string();
    let mut interval_ms = start.interval.saturating_mul(1000).max(1000);
    loop {
        thread::sleep(Duration::from_millis(interval_ms));
        let (status, body) = match http_post(&url, None, Some(&payload)) {
            Ok(pair) => pair,
            Err(_) => {
                eprintln!("{}", term::warn("Network error while polling. Retrying…"));
                continue;
            }
        };
        match parse_device_token(status, &body) {
            DevicePoll::Pending => continue,
            DevicePoll::SlowDown => {
                interval_ms = interval_ms.saturating_add(5000);
            }
            DevicePoll::Denied => return Err("Device login was denied by the user.".into()),
            DevicePoll::Expired => {
                return Err("Device login code expired. Re-run to start a new login.".into())
            }
            DevicePoll::Failed(msg) => return Err(msg),
            DevicePoll::Ready(token) => return Ok(token),
        }
    }
}

#[cfg(not(feature = "native"))]
pub fn login_via_device(
    _base_url: &str,
    _tool: Option<&str>,
    _open_browser: bool,
) -> Result<DeviceToken, String> {
    Err("device login is not available in the browser demo".into())
}

#[cfg(feature = "native")]
pub fn login_via_device(
    base_url: &str,
    tool: Option<&str>,
    open_browser: bool,
) -> Result<DeviceToken, String> {
    let start = start_device_flow(base_url, tool)?;
    print_device_block(&start);
    if open_browser {
        let _ = open_url(&start.verification_uri);
    }
    poll_device_token(base_url, &start)
}

#[derive(Debug, Clone)]
pub struct AcquiredKey {
    pub api_key: String,
    pub management_key: Option<String>,
    pub source: String,
}

/// Priority: --key / ANYROUTER_API_KEY → --device → TTY paste / auto device.
pub fn acquire_api_key(
    flags: &std::collections::HashMap<String, FlagValue>,
    env: &BTreeMap<String, String>,
    base_url: &str,
    tool: Option<&str>,
) -> Result<AcquiredKey, String> {
    if let Some(key) = get_string_flag(flags, "key") {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            return Ok(AcquiredKey {
                api_key: trimmed.to_string(),
                management_key: get_string_flag(flags, "management-key"),
                source: "--key".into(),
            });
        }
    }
    if let Some(key) = env
        .get("ANYROUTER_API_KEY")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return Ok(AcquiredKey {
            api_key: key.to_string(),
            management_key: get_string_flag(flags, "management-key"),
            source: "ANYROUTER_API_KEY".into(),
        });
    }

    let force_device = matches!(flags.get("device"), Some(FlagValue::Bool(true)))
        || matches!(flags.get("device-code"), Some(FlagValue::Bool(true)));
    if force_device {
        let token = login_via_device(base_url, tool, browser_likely_available(env))?;
        return Ok(AcquiredKey {
            api_key: token.api_key,
            management_key: token
                .management_key
                .or_else(|| get_string_flag(flags, "management-key")),
            source: "device code".into(),
        });
    }

    let interactive =
        term::is_interactive() && !matches!(flags.get("yes"), Some(FlagValue::Bool(true)));
    if !interactive {
        return Err(no_key_error());
    }

    eprintln!();
    eprintln!("{}", term::bold("Welcome to AnyRouter"));
    eprintln!(
        "{}",
        term::dim("One key runs Claude Code, Codex, Grok Build and the rest through the gateway.")
    );
    eprintln!();

    if matches!(flags.get("paste"), Some(FlagValue::Bool(true))) {
        let api_key = term::prompt("Paste your AnyRouter API key (sk-ar-...): ")?;
        if api_key.is_empty() {
            return Err("No key entered.".into());
        }
        return Ok(AcquiredKey {
            api_key,
            management_key: get_string_flag(flags, "management-key"),
            source: "paste".into(),
        });
    }

    let open_browser = browser_likely_available(env);
    if open_browser {
        eprintln!(
            "{}",
            term::dim("Opening the device-login page in your browser…")
        );
    }
    match login_via_device(base_url, tool, open_browser) {
        Ok(token) => Ok(AcquiredKey {
            api_key: token.api_key,
            management_key: token
                .management_key
                .or_else(|| get_string_flag(flags, "management-key")),
            source: if open_browser {
                "browser".into()
            } else {
                "device code".into()
            },
        }),
        Err(err) => {
            eprintln!("{}", term::warn(&format!("{err} — paste a key instead.")));
            let api_key = term::prompt("Paste your AnyRouter API key (sk-ar-...): ")?;
            if api_key.is_empty() {
                return Err("No key entered.".into());
            }
            Ok(AcquiredKey {
                api_key,
                management_key: get_string_flag(flags, "management-key"),
                source: "paste".into(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_device_start_reads_fields() {
        let start = parse_device_start(
            r#"{"device_code":"dc_1","user_code":"ABCD-EFGH","verification_uri":"https://anyrouter.dev/cli/device","interval":5,"expires_in":600}"#,
        )
        .unwrap();
        assert_eq!(start.device_code, "dc_1");
        assert_eq!(start.user_code, "ABCD-EFGH");
        assert_eq!(start.interval, 5);
    }

    #[test]
    fn parse_device_token_ready_with_management_key() {
        let poll = parse_device_token(
            200,
            r#"{"key":"sk-ar-v1-secret","management_key":{"secret":"ak_mgmt"}}"#,
        );
        match poll {
            DevicePoll::Ready(t) => {
                assert_eq!(t.api_key, "sk-ar-v1-secret");
                assert_eq!(t.management_key.as_deref(), Some("ak_mgmt"));
            }
            other => panic!("expected ready, got {other:?}"),
        }
    }

    #[test]
    fn parse_device_token_pending_nested_and_flat() {
        assert!(matches!(
            parse_device_token(400, r#"{"error":{"code":"authorization_pending"}}"#),
            DevicePoll::Pending
        ));
        assert!(matches!(
            parse_device_token(400, r#"{"error":"authorization_pending"}"#),
            DevicePoll::Pending
        ));
        assert!(matches!(
            parse_device_token(400, r#"{"error":"slow_down"}"#),
            DevicePoll::SlowDown
        ));
        assert!(matches!(
            parse_device_token(400, r#"{"error":"access_denied"}"#),
            DevicePoll::Denied
        ));
        assert!(matches!(
            parse_device_token(400, r#"{"error":"expired_token"}"#),
            DevicePoll::Expired
        ));
    }

    #[test]
    fn ssh_and_ci_skip_browser() {
        let mut env = BTreeMap::new();
        env.insert("SSH_CONNECTION".into(), "1 2 3 4".into());
        assert!(!browser_likely_available(&env));
        env.clear();
        env.insert("CI".into(), "true".into());
        assert!(!browser_likely_available(&env));
    }
}
