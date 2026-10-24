//! `anyr eval`: one prompt (or a JSONL file of prompts) across several models,
//! then a summary table. One non-streaming `POST /v1/chat/completions` per
//! (prompt, model); a failing model is reported, never swallowed.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use serde_json::{json, Value};

use crate::api::{context, read_file_capped};
use crate::cmd::dispatch::{hint, USAGE};
use crate::cmd::view::render_table;
use crate::http::{error_message, format_usd, http_post_json_headers, join_api, HttpReply};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::catalog_model_id;
use crate::term;

const USAGE_LINE: &str = "Usage: {bin} eval \"<prompt>\" --model a,b,c [--file prompt.md | --prompts cases.jsonl] [--max-tokens N] [--concurrency N] [--full] [--json]";
const DEFAULT_CONCURRENCY: usize = 4;
const MAX_CONCURRENCY: usize = 32;
/// Characters of an answer shown in table mode without `--full`.
const PREVIEW_CHARS: usize = 600;

/// One finished request for a (prompt, model) pair.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outcome {
    pub(crate) prompt_index: usize,
    pub(crate) prompt: String,
    pub(crate) model: String,
    pub(crate) answer: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) latency_ms: u64,
    pub(crate) tokens_in: Option<u64>,
    pub(crate) tokens_out: Option<u64>,
    pub(crate) cost: Option<f64>,
    pub(crate) error: Option<String>,
}

impl Outcome {
    fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// `a, b,,c` -> `["a","b","c"]`; repeats keep their first position.
pub(crate) fn parse_models(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in raw.split(',').map(catalog_model_id) {
        if !m.is_empty() && !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

/// One prompt per non-blank line: `{"prompt": "..."}`. Errors name the line.
pub(crate) fn parse_prompts_jsonl(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let n = i + 1;
        let v: Value = serde_json::from_str(line)
            .map_err(|e| format!("prompts line {n}: invalid JSON ({e})"))?;
        match v.get("prompt").and_then(Value::as_str).map(str::trim) {
            Some(p) if !p.is_empty() => out.push(p.to_string()),
            _ => {
                return Err(format!(
                    "prompts line {n}: needs a non-empty \"prompt\" string"
                ))
            }
        }
    }
    if out.is_empty() {
        return Err("prompts file has no prompts".into());
    }
    Ok(out)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
}

/// Turn one HTTP reply into an `Outcome`. Pure, so it is unit tested.
pub(crate) fn outcome_from_reply(
    prompt_index: usize,
    prompt: &str,
    model: &str,
    latency_ms: u64,
    reply: Result<HttpReply, String>,
) -> Outcome {
    let mut o = Outcome {
        prompt_index,
        prompt: prompt.to_string(),
        model: model.to_string(),
        answer: None,
        provider: None,
        latency_ms,
        tokens_in: None,
        tokens_out: None,
        cost: None,
        error: None,
    };
    let (status, headers, body) = match reply {
        Ok(r) => r,
        Err(e) => {
            o.error = Some(e);
            return o;
        }
    };
    if !(200..300).contains(&status) {
        o.error = Some(error_message(status, &body));
        return o;
    }
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        o.error = Some("response was not JSON".into());
        return o;
    };
    o.provider = header(&headers, "x-anyrouter-provider").map(str::to_string);
    o.tokens_in = v.pointer("/usage/prompt_tokens").and_then(Value::as_u64);
    o.tokens_out = v
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_u64);
    o.cost = header(&headers, "x-anyrouter-credits-charged")
        .and_then(|c| c.parse().ok())
        .or_else(|| v.pointer("/usage/cost").and_then(Value::as_f64));
    match v
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
    {
        Some(a) => o.answer = Some(a.to_string()),
        None => o.error = Some("response had no answer text".into()),
    }
    o
}

fn run_one(
    url: &str,
    key: &str,
    index: usize,
    prompt: &str,
    model: &str,
    max_tokens: Option<u32>,
) -> Outcome {
    let mut body = json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": false,
    });
    if let Some(n) = max_tokens {
        body["max_tokens"] = json!(n);
    }
    let start = Instant::now();
    let reply = http_post_json_headers(url, key, &body.to_string());
    let ms = start.elapsed().as_millis() as u64;
    outcome_from_reply(index, prompt, model, ms, reply)
}

/// Run every (prompt, model) pair on `concurrency` threads. Results come back
/// in input order (prompts outer, models inner) whatever finishes first.
pub(crate) fn run_all<F>(
    prompts: &[String],
    models: &[String],
    concurrency: usize,
    call: F,
) -> Vec<Outcome>
where
    F: Fn(usize, &str, &str) -> Outcome + Sync,
{
    let jobs: Vec<(usize, &str, &str)> = prompts
        .iter()
        .enumerate()
        .flat_map(|(i, p)| models.iter().map(move |m| (i, p.as_str(), m.as_str())))
        .collect();
    let slots: Vec<Mutex<Option<Outcome>>> = jobs.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..concurrency.min(jobs.len()).max(1) {
            s.spawn(|| loop {
                let n = next.fetch_add(1, Ordering::SeqCst);
                let Some(&(i, p, m)) = jobs.get(n) else { break };
                let outcome = call(i, p, m);
                *slots[n].lock().unwrap() = Some(outcome);
            });
        }
    });
    slots
        .into_iter()
        .filter_map(|s| s.into_inner().unwrap())
        .collect()
}

/// Rows for the summary table: model, provider, latency, tokens in/out, cost, status.
pub(crate) fn summary_rows(outcomes: &[Outcome]) -> Vec<Vec<String>> {
    let num = |n: Option<u64>| n.map_or("-".into(), |n| n.to_string());
    outcomes
        .iter()
        .map(|o| {
            vec![
                o.model.clone(),
                o.provider.clone().unwrap_or_else(|| "-".into()),
                format!("{}ms", o.latency_ms),
                num(o.tokens_in),
                num(o.tokens_out),
                o.cost.map_or("-".into(), format_usd),
                if o.ok() { "ok" } else { "error" }.into(),
            ]
        })
        .collect()
}

fn truncate(answer: &str, full: bool) -> String {
    let answer = answer.trim();
    if full || answer.chars().count() <= PREVIEW_CHARS {
        return answer.to_string();
    }
    let cut: String = answer.chars().take(PREVIEW_CHARS).collect();
    let more = answer.chars().count() - PREVIEW_CHARS;
    format!("{}…\n[{more} more characters, use --full]", cut.trim_end())
}

pub(crate) fn json_line(o: &Outcome) -> Value {
    json!({
        "prompt_index": o.prompt_index,
        "prompt": o.prompt,
        "model": o.model,
        "status": if o.ok() { "ok" } else { "error" },
        "answer": o.answer,
        "provider": o.provider,
        "latency_ms": o.latency_ms,
        "tokens_in": o.tokens_in,
        "tokens_out": o.tokens_out,
        "cost": o.cost,
        "error": o.error,
    })
}

fn render(outcomes: &[Outcome], prompts: &[String], full: bool, tty: bool) -> String {
    let mut out = String::new();
    for (i, prompt) in prompts.iter().enumerate() {
        let group: Vec<Outcome> = outcomes
            .iter()
            .filter(|o| o.prompt_index == i)
            .cloned()
            .collect();
        if prompts.len() > 1 {
            let line: String = prompt
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(70)
                .collect();
            out += &format!("{}\n\n", term::bold(&format!("Prompt {}: {line}", i + 1)));
        }
        for o in &group {
            let via = o
                .provider
                .as_deref()
                .map(|p| format!(" via {p}"))
                .unwrap_or_default();
            out += &format!("{}\n", term::bold(&format!("{}{via}", o.model)));
            match (&o.answer, &o.error) {
                (_, Some(e)) => out += &format!("{} {e}\n\n", term::err("error:")),
                (Some(a), None) => out += &format!("{}\n\n", truncate(a, full)),
                (None, None) => out.push('\n'),
            }
        }
        out += &render_table(
            &[
                "model", "provider", "latency", "in", "out", "cost", "status",
            ],
            &summary_rows(&group),
            tty,
        );
        if i + 1 < prompts.len() {
            out.push('\n');
        }
    }
    out
}

fn positive(raw: Option<String>, flag: &str, max: usize) -> Result<Option<usize>, String> {
    let Some(raw) = raw else { return Ok(None) };
    match raw.parse::<usize>() {
        Ok(n) if (1..=max).contains(&n) => Ok(Some(n)),
        _ => Err(format!("{USAGE}--{flag} must be a number from 1 to {max}.")),
    }
}

fn load_prompts(parsed: &ParsedArgs) -> Result<Vec<String>, String> {
    let positional = parsed.passthrough.join(" ");
    let positional = positional.trim();
    let file = get_string_flag(&parsed.flags, "file");
    let jsonl = get_string_flag(&parsed.flags, "prompts");
    let sources = [!positional.is_empty(), file.is_some(), jsonl.is_some()];
    if sources.iter().filter(|s| **s).count() != 1 {
        return Err(hint(&format!(
            "{USAGE_LINE}\nGive exactly one of: a prompt, --file, --prompts."
        )));
    }
    if let Some(path) = jsonl {
        return parse_prompts_jsonl(&read_file_capped(&path)?);
    }
    if let Some(path) = file {
        let text = read_file_capped(&path)?;
        if text.trim().is_empty() {
            return Err(format!("{path} is empty"));
        }
        return Ok(vec![text.trim().to_string()]);
    }
    Ok(vec![positional.to_string()])
}

pub(crate) fn run_eval(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let models = get_string_flag(&parsed.flags, "model")
        .map(|m| parse_models(&m))
        .unwrap_or_default();
    if models.is_empty() {
        return Err(hint(&format!(
            "{USAGE_LINE}\n--model needs at least one model id."
        )));
    }
    let max_tokens = positive(
        get_string_flag(&parsed.flags, "max-tokens"),
        "max-tokens",
        u32::MAX as usize,
    )?
    .map(|n| n as u32);
    let concurrency = positive(
        get_string_flag(&parsed.flags, "concurrency"),
        "concurrency",
        MAX_CONCURRENCY,
    )?
    .unwrap_or(DEFAULT_CONCURRENCY);
    let prompts = load_prompts(parsed)?;
    let ctx = context(parsed, env);
    let key = ctx.key()?.to_string();
    let url = join_api(&ctx.base, "/v1/chat/completions");

    let outcomes = run_all(&prompts, &models, concurrency, |i, p, m| {
        run_one(&url, &key, i, p, m, max_tokens)
    });

    if ctx.json {
        for o in &outcomes {
            println!("{}", json_line(o));
        }
    } else {
        use std::io::IsTerminal;
        print!(
            "{}",
            render(
                &outcomes,
                &prompts,
                parsed.flag_true("full"),
                std::io::stdout().is_terminal()
            )
        );
    }
    let failed = outcomes.iter().filter(|o| !o.ok()).count();
    if failed > 0 {
        eprintln!(
            "{} {failed} of {} requests failed",
            term::err("error:"),
            outcomes.len()
        );
        return Ok(1);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16, headers: &[(&str, &str)], body: &str) -> Result<HttpReply, String> {
        Ok((
            status,
            headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body.to_string(),
        ))
    }

    #[test]
    fn models_split_trim_and_dedupe_in_given_order() {
        assert_eq!(parse_models(" b/x , a/y,,b/x ,c/z"), ["b/x", "a/y", "c/z"]);
        assert!(parse_models(" , ").is_empty());
    }

    #[test]
    fn jsonl_bad_line_errors_with_its_line_number() {
        let ok = parse_prompts_jsonl("{\"prompt\":\"one\"}\n\n{\"prompt\":\" two \"}\n").unwrap();
        assert_eq!(ok, ["one", "two"]);
        let e = parse_prompts_jsonl("{\"prompt\":\"one\"}\nnot json\n").unwrap_err();
        assert!(e.starts_with("prompts line 2:"), "{e}");
        let e = parse_prompts_jsonl("{\"prompt\":\"one\"}\n{\"text\":\"x\"}\n").unwrap_err();
        assert!(e.starts_with("prompts line 2:"), "{e}");
        assert!(parse_prompts_jsonl("\n\n").is_err());
    }

    #[test]
    fn success_reads_usage_provider_and_cost_header() {
        let body = r#"{"choices":[{"message":{"content":"hi"}}],"usage":{"prompt_tokens":5,"completion_tokens":7,"cost":0.9}}"#;
        let o = outcome_from_reply(
            0,
            "p",
            "m",
            12,
            reply(
                200,
                &[
                    ("x-anyrouter-provider", "nvidia"),
                    ("x-anyrouter-credits-charged", "0.0021"),
                ],
                body,
            ),
        );
        assert!(o.ok());
        assert_eq!(o.answer.as_deref(), Some("hi"));
        assert_eq!(o.provider.as_deref(), Some("nvidia"));
        assert_eq!((o.tokens_in, o.tokens_out), (Some(5), Some(7)));
        // The charged header wins over usage.cost.
        assert_eq!(o.cost, Some(0.0021));
        let row = &summary_rows(&[o])[0];
        assert_eq!(row, &["m", "nvidia", "12ms", "5", "7", "$0.0021", "ok"]);
    }

    #[test]
    fn http_error_and_transport_error_become_error_rows() {
        let o = outcome_from_reply(
            0,
            "p",
            "m",
            1,
            reply(402, &[], r#"{"error":{"message":"no credits"}}"#),
        );
        assert_eq!(o.error.as_deref(), Some("HTTP 402: no credits"));
        assert_eq!(summary_rows(&[o])[0][6], "error");
        let o = outcome_from_reply(0, "p", "m", 1, Err("Could not reach AnyRouter: x".into()));
        assert!(!o.ok());
        let o = outcome_from_reply(0, "p", "m", 1, reply(200, &[], r#"{"choices":[]}"#));
        assert!(
            !o.ok(),
            "a 200 without an answer is an error, not a blank success"
        );
    }

    #[test]
    fn run_all_keeps_input_order_whatever_finishes_first() {
        let prompts = vec!["p0".to_string(), "p1".to_string()];
        let models: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let out = run_all(&prompts, &models, 8, |i, p, m| {
            // Earlier models sleep longest, so completion order is reversed.
            let delay = 30 - 10 * ["a", "b", "c"].iter().position(|x| *x == m).unwrap() as u64;
            std::thread::sleep(std::time::Duration::from_millis(delay));
            outcome_from_reply(
                i,
                p,
                m,
                0,
                reply(200, &[], r#"{"choices":[{"message":{"content":"x"}}]}"#),
            )
        });
        let order: Vec<(usize, &str)> = out
            .iter()
            .map(|o| (o.prompt_index, o.model.as_str()))
            .collect();
        assert_eq!(
            order,
            [(0, "a"), (0, "b"), (0, "c"), (1, "a"), (1, "b"), (1, "c")]
        );
    }

    #[test]
    fn long_answers_are_cut_unless_full() {
        let long = "x".repeat(PREVIEW_CHARS + 50);
        assert!(truncate(&long, false).contains("50 more characters"));
        assert_eq!(truncate(&long, true), long);
        assert_eq!(truncate("short", false), "short");
    }
}
