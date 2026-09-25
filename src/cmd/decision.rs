use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Read};

use serde_json::{Map, Value};

use crate::config::get_active_profile;
use crate::http::create_decision;
use crate::key::{
    load_config_if_present, mask_api_key, no_key_error, resolve_api_key, resolve_base_url,
};
use crate::parse::{get_string_flag, ParsedArgs};

use crate::cmd::dispatch::config_path;

/// Run the native Decisions API command. This deliberately builds the
/// `{model, state, questions}` envelope instead of reusing a chat launcher.
pub(crate) fn run_decision(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let stdin_json = read_stdin_json(parsed)?;
    let body = build_decision_body(parsed, stdin_json.as_deref())?;

    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let profile = match existing.as_ref() {
        Some(config) => {
            let requested_profile = get_string_flag(&parsed.flags, "profile");
            let env_profile = env
                .get("ANYROUTER_PROFILE")
                .map(|name| name.trim())
                .filter(|name| !name.is_empty());
            if requested_profile.is_some() || env_profile.is_some() {
                Some(get_active_profile(
                    config,
                    requested_profile.as_deref(),
                    env,
                )?)
            } else {
                config.profiles.get(&config.active_profile)
            }
        }
        None => None,
    };
    let api_key = resolve_api_key(&parsed.flags, env, profile).ok_or_else(no_key_error)?;
    let base_url = resolve_base_url(&parsed.flags, profile);
    let response = create_decision(&base_url, &api_key, &body)?;

    let rendered = if parsed.flag_true("json") {
        serde_json::to_string(&response)
    } else {
        serde_json::to_string_pretty(&response)
    }
    .map_err(|err| format!("Could not render decision response: {err}"))?;
    let rendered = rendered.replace(&api_key, &mask_api_key(Some(&api_key)));
    println!("{rendered}");
    Ok(0)
}

/// Read stdin only when it can contribute fields. A complete set of flags is
/// enough on its own, and an interactive invocation should fail with a useful
/// input error instead of waiting for a JSON document.
fn read_stdin_json(parsed: &ParsedArgs) -> Result<Option<String>, String> {
    let all_fields_are_flags = ["model", "state", "questions"]
        .iter()
        .all(|name| get_string_flag(&parsed.flags, name).is_some());
    if !parsed.flag_true("stdin") && (all_fields_are_flags || io::stdin().is_terminal()) {
        return Ok(None);
    }

    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|err| format!("Could not read decision JSON from stdin: {err}"))?;
    Ok((!input.trim().is_empty()).then_some(input))
}

/// Merge a stdin object with flags. Flags win, which makes it possible to keep
/// a reusable JSON state/question document while changing only `--model`.
pub(crate) fn build_decision_body(
    parsed: &ParsedArgs,
    stdin_json: Option<&str>,
) -> Result<Value, String> {
    let mut fields = Map::new();

    if let Some(raw) = stdin_json {
        let value: Value = serde_json::from_str(raw)
            .map_err(|err| format!("Invalid decision JSON on stdin: {err}"))?;
        let object = value.as_object().ok_or_else(|| {
            "Decision JSON on stdin must be an object with model, state, and questions.".to_string()
        })?;
        for name in ["model", "state", "questions"] {
            if let Some(value) = object.get(name) {
                fields.insert(name.to_string(), value.clone());
            }
        }
    }

    if let Some(model) = get_string_flag(&parsed.flags, "model") {
        fields.insert("model".into(), Value::String(model));
    }
    if let Some(state) = get_string_flag(&parsed.flags, "state") {
        fields.insert("state".into(), parse_state_value(&state)?);
    }
    if let Some(questions) = get_string_flag(&parsed.flags, "questions") {
        let value: Value = serde_json::from_str(&questions)
            .map_err(|err| format!("Invalid --questions JSON: {err}"))?;
        fields.insert("questions".into(), value);
    }

    let model =
        match fields.get("model") {
            Some(Value::String(model)) if !model.trim().is_empty() => model.trim().to_string(),
            _ => return Err(
                "Decision input needs a model. Pass --model <id> or include model in stdin JSON."
                    .into(),
            ),
        };
    let state = fields.get("state").cloned().ok_or_else(|| {
        "Decision input needs state. Pass --state or include state in stdin JSON.".to_string()
    })?;
    if state.is_null() {
        return Err("Decision state cannot be null.".into());
    }
    let questions = fields.get("questions").cloned().ok_or_else(|| {
        "Decision input needs questions. Pass --questions or include questions in stdin JSON."
            .to_string()
    })?;
    let questions = match questions {
        Value::Object(map) if !map.is_empty() => Value::Object(map),
        Value::Array(_) => {
            return Err(
                "questions must be a non-empty object map of noul|choice|score question definitions, not an array"
                    .into(),
            )
        }
        _ => {
            return Err(
                "questions must be a non-empty object map of noul|choice|score question definitions"
                    .into(),
            )
        }
    };

    let mut body = Map::new();
    body.insert("model".into(), Value::String(model));
    body.insert("state".into(), state);
    body.insert("questions".into(), questions);
    Ok(Value::Object(body))
}

fn parse_state_value(raw: &str) -> Result<Value, String> {
    match serde_json::from_str(raw) {
        Ok(value) => Ok(value),
        Err(err) if looks_like_json(raw) => Err(format!("Invalid --state JSON: {err}")),
        Err(_) => Ok(Value::String(raw.to_string())),
    }
}

fn looks_like_json(raw: &str) -> bool {
    let trimmed = raw.trim_start();
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    matches!(first, '{' | '[' | '"' | 't' | 'f' | 'n' | '-' | '0'..='9')
}

/// The current public Decisions catalog. Keep aliases here so a chat launcher
/// cannot silently turn a known `systemone` model into a chat request. The API
/// remains the source of truth for new models; the server rejects unknown or
/// non-Decisions ids on `/v1/decisions`.
pub(crate) fn is_systemone_model(model: &str) -> bool {
    let mut routing = crate::config::RoutingConstraints::default();
    let id = crate::spawn::apply_model_id_routing(model, &mut routing);
    id == "anyrouter/decision"
        || id == "typesafe/jev"
        || id.starts_with("typesafe/jev-")
        || id == "fastino/gliner2.5-multi-v1"
        || id == "fastino/gliner2.5-decide"
        || id.starts_with("fastino/gliner2.5-")
}

pub(crate) fn chat_model_error(model: &str, tool_name: &str) -> String {
    let bin = crate::help::invoked_bin();
    format!(
        "Model \"{model}\" is a Decisions model, not a chat model. Refusing to launch it as {tool_name}. Use `{bin} decision --model {model} --state <text|json> --questions <json>` (POST /api/v1/decisions)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_cli_args;

    fn parsed(args: &[&str]) -> ParsedArgs {
        parse_cli_args(args).expect("parse decision args")
    }

    #[test]
    fn flags_build_the_native_decisions_envelope() {
        let parsed = parsed(&[
            "decision",
            "--model",
            "typesafe/jev",
            "--state",
            "Help! Payouts are failing.",
            "--questions",
            r#"{"is_urgent":{"type":"noul","instructions":"Is this urgent?"}}"#,
        ]);
        let body = build_decision_body(&parsed, None).expect("build body");
        assert_eq!(body["model"], "typesafe/jev");
        assert_eq!(body["state"], "Help! Payouts are failing.");
        assert_eq!(body["questions"]["is_urgent"]["type"], "noul");
        assert_eq!(body.as_object().unwrap().len(), 3);
    }

    #[test]
    fn stdin_fields_merge_and_flags_take_precedence() {
        let parsed = parsed(&["decision", "--model", "fastino/gliner2.5-multi-v1"]);
        let body = build_decision_body(
            &parsed,
            Some(r#"{"model":"typesafe/jev","state":{"order":"42"},"questions":{"intent":{"type":"choice","criteria":["a"]}}}"#),
        )
        .expect("build body");
        assert_eq!(body["model"], "fastino/gliner2.5-multi-v1");
        assert_eq!(body["state"]["order"], "42");
        assert_eq!(body["questions"]["intent"]["type"], "choice");
    }

    #[test]
    fn structured_state_and_question_map_are_supported() {
        let parsed = parsed(&[
            "decision",
            "--model",
            "anyrouter/decision",
            "--state",
            r#"{"order":{"status":"failed"}}"#,
            "--questions",
            r#"{"urgent":{"type":"noul"}}"#,
        ]);
        let body = build_decision_body(&parsed, None).expect("build body");
        assert_eq!(body["state"]["order"]["status"], "failed");
    }

    #[test]
    fn question_arrays_are_rejected_before_network_access() {
        let parsed = parsed(&[
            "decision",
            "--model",
            "typesafe/jev",
            "--state",
            "hello",
            "--questions",
            r#"[{"type":"noul"}]"#,
        ]);
        let err = build_decision_body(&parsed, None).unwrap_err();
        assert!(err.contains("not an array"), "{err}");
    }

    #[test]
    fn known_systemone_ids_are_not_chat_models() {
        for model in [
            "typesafe/jev",
            "typesafe/jev-latest",
            "typesafe/jev-1.13.0",
            "fastino/gliner2.5-multi-v1",
            "anyrouter/decision",
            "anyrouter/decision[1m]",
            "fastino/gliner2.5-multi-v1:exacto",
        ] {
            assert!(is_systemone_model(model), "{model}");
        }
        assert!(!is_systemone_model("anthropic/claude-sonnet-4.6"));
        assert!(!is_systemone_model("anyrouter/auto"));
    }
}
