//! `anyr task "<request>"`: one model writes a numbered plan, a second model
//! carries it out. Two `POST /v1/chat/completions` calls, plan first, with the
//! plan text fed into the implementation prompt.

use std::collections::BTreeMap;
use std::io::{self, Read};

use serde_json::{json, Value};

use crate::api::{confirm, context, get, Ctx};
use crate::chat::stream_reply_usage;
use crate::cmd::dispatch::{config_path, hint};
use crate::cmd::view::print_json;
use crate::http::{error_message, format_usd, http_post_json_headers, http_post_stream, join_api};
use crate::key::load_config_if_present;
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::catalog_model_id;
use crate::term;

const USAGE_LINE: &str =
    "Usage: {bin} task \"<request>\" [--plan-model <id>] [--do-model <id>] [--yes] [--json]";

const PLAN_SYSTEM: &str = "You are a planning assistant. Write a numbered, step-by-step plan for the task. Be concrete and brief. Do not carry out the plan.";
const DO_SYSTEM: &str = "You are an implementation assistant. Carry out the task by following the given plan exactly, step by step, and give the final result.";

/// USD per million tokens, from the catalog.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Price {
    input: f64,
    output: f64,
}

impl Price {
    fn from_model(m: &Value) -> Option<Price> {
        Some(Price {
            input: m.pointer("/pricing/input_per_1m")?.as_f64()?,
            output: m.pointer("/pricing/output_per_1m")?.as_f64()?,
        })
    }

    fn cost(self, prompt: u64, completion: u64) -> f64 {
        (self.input * prompt as f64 + self.output * completion as f64) / 1_000_000.0
    }
}

/// One finished phase.
#[derive(Debug, Clone, PartialEq)]
struct Phase {
    text: String,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    cost: Option<f64>,
}

impl Phase {
    fn usage_json(&self) -> Value {
        json!({"prompt_tokens": self.prompt_tokens, "completion_tokens": self.completion_tokens})
    }
}

fn plan_messages(task: &str) -> Value {
    json!([
        {"role": "system", "content": PLAN_SYSTEM},
        {"role": "user", "content": task},
    ])
}

fn do_messages(task: &str, plan: &str) -> Value {
    json!([
        {"role": "system", "content": DO_SYSTEM},
        {"role": "user", "content": format!("Task:\n{task}\n\nPlan:\n{plan}\n\nCarry out the plan now.")},
    ])
}

fn price_of(ctx: &Ctx, model: &str) -> Option<Price> {
    get(
        ctx,
        &format!("/v1/models/{}", crate::api::encode(model)),
        false,
    )
    .ok()
    .and_then(|m| Price::from_model(&m))
}

fn price_text(model: &str, price: Option<Price>) -> String {
    match price {
        Some(p) => format!(
            "{model} ({} in / {} out per 1M tokens)",
            format_usd(p.input),
            format_usd(p.output)
        ),
        None => format!("{model} (price unknown)"),
    }
}

/// Non-streaming call for `--json`.
fn call_json(
    url: &str,
    key: &str,
    model: &str,
    messages: &Value,
    price: Option<Price>,
) -> Result<Phase, String> {
    let body = json!({"model": model, "messages": messages, "stream": false});
    let (status, headers, text) = http_post_json_headers(url, key, &body.to_string())?;
    if !(200..300).contains(&status) {
        return Err(error_message(status, &text));
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| "response was not JSON".to_string())?;
    let answer = v
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or("response had no answer text")?
        .to_string();
    let prompt_tokens = v.pointer("/usage/prompt_tokens").and_then(Value::as_u64);
    let completion_tokens = v
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_u64);
    let charged = headers
        .iter()
        .find(|(k, _)| k == "x-anyrouter-credits-charged")
        .and_then(|(_, c)| c.parse().ok());
    let cost = charged.or_else(|| match (price, prompt_tokens, completion_tokens) {
        (Some(p), Some(i), Some(o)) => Some(p.cost(i, o)),
        _ => None,
    });
    Ok(Phase {
        text: answer,
        prompt_tokens,
        completion_tokens,
        cost,
    })
}

/// Streaming call: text goes to stdout as it arrives.
fn call_stream(
    url: &str,
    key: &str,
    model: &str,
    messages: &Value,
    price: Option<Price>,
) -> Result<Phase, String> {
    let body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    let (status, mut reader) = http_post_stream(url, key, &body.to_string())?;
    if !(200..300).contains(&status) {
        let mut text = String::new();
        let _ = reader.read_to_string(&mut text);
        return Err(error_message(status, &text));
    }
    let (text, usage) = stream_reply_usage(io::BufReader::new(reader), &mut io::stdout().lock())?;
    println!();
    let cost = match (price, usage) {
        (Some(p), Some((i, o))) => Some(p.cost(i, o)),
        _ => None,
    };
    Ok(Phase {
        text,
        prompt_tokens: usage.map(|u| u.0),
        completion_tokens: usage.map(|u| u.1),
        cost,
    })
}

fn sum_cost(a: &Phase, b: &Phase) -> Option<f64> {
    Some(a.cost? + b.cost?)
}

fn token_line(label: &str, p: &Phase) -> String {
    match (p.prompt_tokens, p.completion_tokens) {
        (Some(i), Some(o)) => format!("{label}: {i} in / {o} out tokens"),
        _ => format!("{label}: token usage not reported"),
    }
}

pub(crate) fn run_task(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let task = parsed.passthrough.join(" ");
    let task = task.trim();
    if task.is_empty() {
        return Err(hint(USAGE_LINE));
    }
    let ctx = context(parsed, env);
    let key = ctx.key()?.to_string();
    let cfg = load_config_if_present(&config_path(parsed, env));
    let default_model = cfg
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile))
        .map(|p| p.default_model().to_string())
        .unwrap_or_else(|| crate::config::DEFAULT_MODEL.to_string());
    let pick = |flag: &str| {
        catalog_model_id(
            &get_string_flag(&parsed.flags, flag).unwrap_or_else(|| default_model.clone()),
        )
    };
    let (plan_model, do_model) = (pick("plan-model"), pick("do-model"));

    let plan_price = price_of(&ctx, &plan_model);
    let do_price = price_of(&ctx, &do_model);
    confirm(
        parsed,
        &format!(
            "Plan with {}, then implement with {}. This makes two model calls. Run?",
            price_text(&plan_model, plan_price),
            price_text(&do_model, do_price)
        ),
    )?;

    let url = join_api(&ctx.base, "/v1/chat/completions");
    let json_mode = parsed.flag_true("json");
    let call = |model: &str, messages: &Value, price: Option<Price>| {
        if json_mode {
            call_json(&url, &key, model, messages, price)
        } else {
            call_stream(&url, &key, model, messages, price)
        }
    };

    if !json_mode {
        eprintln!("{}", term::dim(&format!("Plan ({plan_model})")));
    }
    let plan = call(&plan_model, &plan_messages(task), plan_price)
        .map_err(|e| format!("planning failed: {e}"))?;
    if !json_mode {
        eprintln!("{}", term::dim(&format!("Implement ({do_model})")));
    }
    let done = call(&do_model, &do_messages(task, &plan.text), do_price)
        .map_err(|e| format!("implementation failed: {e}"))?;
    let cost = sum_cost(&plan, &done);

    if json_mode {
        print_json(&json!({
            "task": task,
            "plan_model": plan_model,
            "do_model": do_model,
            "plan": plan.text,
            "result": done.text,
            "usage": {"plan": plan.usage_json(), "implement": done.usage_json()},
            "cost": cost,
        }));
    } else {
        eprintln!("{}", term::dim(&token_line("plan", &plan)));
        eprintln!("{}", term::dim(&token_line("implement", &done)));
        if let Some(c) = cost {
            eprintln!("{}", term::dim(&format!("cost: {}", format_usd(c))));
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn do_prompt_carries_task_and_plan() {
        let m = do_messages("dedupe files", "1. hash\n2. delete");
        let user = m[1]["content"].as_str().unwrap();
        assert!(user.contains("dedupe files") && user.contains("1. hash\n2. delete"));
    }

    #[test]
    fn price_cost_is_per_million_tokens() {
        let p = Price {
            input: 2.0,
            output: 4.0,
        };
        assert!((p.cost(500_000, 250_000) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn price_needs_both_sides() {
        assert!(Price::from_model(&json!({"pricing": {"input_per_1m": 1.0}})).is_none());
    }
}
