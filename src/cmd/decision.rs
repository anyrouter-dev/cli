//! `anyr decision` — POST a structured decision request to AnyRouter.
//!
//! Decisions are not chat-agent launches. The command sends the native
//! `{ state, model, questions }` contract to `/v1/decisions` and prints the
//! structured response without routing through Claude/Codex/OpenCode.

use std::collections::BTreeMap;
use std::io::{self, Read};

use serde_json::Value;

use crate::cmd::dispatch::{config_path, hint};
use crate::config::strip_context_window_suffix;
use crate::http::{http_post, join_api};
use crate::key::{
    active_profile, load_config_if_present, no_key_error, resolve_api_key, resolve_base_url,
};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::{catalog_model_id, is_virtual_preset};

/// Run `anyr decision`.
pub(crate) fn run_decision(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let config = load_config_if_present(&path);
    let profile = config
        .as_ref()
        .and_then(|cfg| active_profile(cfg, &parsed.flags, env).ok());

    let requested_model = get_string_flag(&parsed.flags, "model")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            hint("Usage: {bin} decision --model <id> --questions <json> [--state <json>]")
        })?;
    let model = strip_context_window_suffix(&catalog_model_id(&requested_model)).to_string();

    if is_virtual_preset(&model) {
        return Err(
            "Virtual chat presets are not valid Decisions models. Use a concrete model id \
             such as typesafe/jev, fastino/gliner2.5-multi-v1, or anyrouter/decision."
                .into(),
        );
    }

    let body = decision_body(parsed, &model)?;
    let key = resolve_api_key(&parsed.flags, env, profile).ok_or_else(no_key_error)?;
    let base = resolve_base_url(&parsed.flags, profile);
    let url = join_api(&base, "/v1/decisions");
    let (status, response) = http_post(&url, Some(&key), Some(&body))?;

    if !(200..300).contains(&status) {
        return Err(extract_error(&response)
            .unwrap_or_else(|| format!("Decision request failed (HTTP {status})")));
    }

    if parsed.flag_true("json") {
        let value: Value = serde_json::from_str(&response)
            .map_err(|error| format!("Invalid JSON response: {error}"))?;
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
        );
    } else {
        println!("{response}");
    }

    Ok(0)
}

/// Build a bounded Decisions request from flags or one JSON object on stdin.
fn decision_body(parsed: &ParsedArgs, model: &str) -> Result<String, String> {
    if parsed.flag_true("stdin") {
        if parsed.flags.contains_key("state") || parsed.flags.contains_key("questions") {
            return Err("Use either --stdin or --state/--questions, not both.".into());
        }
        return read_stdin_body(model);
    }

    let state = match get_string_flag(&parsed.flags, "state") {
        Some(raw) => parse_state(&raw),
        None => Value::String(String::new()),
    };
    let questions = get_string_flag(&parsed.flags, "questions")
        .ok_or_else(|| {
            "Missing --questions JSON (or pipe a full request with --stdin).".to_string()
        })
        .and_then(|raw| parse_json(&raw, "questions"))?;
    validate_questions(&questions)?;

    serde_json::to_string(&serde_json::json!({
        "model": model,
        "state": state,
        "questions": questions,
    }))
    .map_err(|error| format!("Could not encode decision request: {error}"))
}

fn read_stdin_body(model: &str) -> Result<String, String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| format!("Could not read stdin: {error}"))?;
    let mut value: Value = serde_json::from_str(input.trim())
        .map_err(|error| format!("Invalid JSON on stdin: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "Decision request on stdin must be a JSON object.".to_string())?;

    if let Some(existing) = object.get("model") {
        if existing.as_str() != Some(model) {
            return Err("The --model flag must match the model in the stdin JSON.".into());
        }
    } else {
        object.insert("model".into(), Value::String(model.into()));
    }
    let questions = object
        .get("questions")
        .cloned()
        .ok_or_else(|| "Decision request JSON must include questions.".to_string())?;
    validate_questions(&questions)?;
    object.insert("questions".into(), questions);

    serde_json::to_string(&value)
        .map_err(|error| format!("Could not encode decision request: {error}"))
}

fn parse_state(raw: &str) -> Value {
    serde_json::from_str(raw.trim()).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn parse_json(raw: &str, field: &str) -> Result<Value, String> {
    serde_json::from_str(raw.trim()).map_err(|error| format!("Invalid JSON for --{field}: {error}"))
}

fn validate_questions(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "--questions must be a non-empty JSON object.".to_string())?;
    if object.is_empty() {
        return Err("--questions must contain at least one question.".into());
    }
    Ok(())
}

fn extract_error(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value
        .get("error")
        .and_then(|error| {
            error
                .as_str()
                .or_else(|| error.get("message").and_then(Value::as_str))
        })
        .or_else(|| value.get("message").and_then(Value::as_str))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::FlagValue;
    use std::collections::HashMap;

    fn parsed(flags: &[(&str, FlagValue)], passthrough: &[&str]) -> ParsedArgs {
        ParsedArgs {
            command: "decision".into(),
            flags: flags
                .iter()
                .map(|(key, value)| ((*key).to_string(), value.clone()))
                .collect::<HashMap<_, _>>(),
            passthrough: passthrough
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        }
    }

    #[test]
    fn builds_body_from_json_flags() {
        let args = parsed(
            &[
                ("state", FlagValue::Value("hello".into())),
                (
                    "questions",
                    FlagValue::Value(r#"{"urgent":{"type":"noul"}}"#.into()),
                ),
            ],
            &[],
        );
        let value: Value =
            serde_json::from_str(&decision_body(&args, "typesafe/jev").unwrap()).unwrap();
        assert_eq!(value["model"], "typesafe/jev");
        assert_eq!(value["state"], "hello");
        assert_eq!(value["questions"]["urgent"]["type"], "noul");
    }

    #[test]
    fn rejects_empty_questions() {
        let args = parsed(&[("questions", FlagValue::Value("{}".into()))], &[]);
        assert!(decision_body(&args, "typesafe/jev")
            .unwrap_err()
            .contains("at least one"));
    }

    #[test]
    fn rejects_non_object_questions() {
        let args = parsed(&[("questions", FlagValue::Value("[]".into()))], &[]);
        assert!(decision_body(&args, "typesafe/jev")
            .unwrap_err()
            .contains("JSON object"));
    }

    #[test]
    fn extracts_nested_error() {
        assert_eq!(
            extract_error(r#"{"error":{"message":"invalid key"}}"#).as_deref(),
            Some("invalid key")
        );
    }
}
