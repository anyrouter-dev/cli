//! `anyr chat [prompt]` — minimal streaming chat over `POST /v1/chat/completions`.
//! One-shot with a prompt argument (or piped stdin); a REPL on a terminal.

use std::collections::BTreeMap;
use std::io::{self, BufRead, Read, Write};

use serde_json::{json, Value};

use crate::cmd::dispatch::config_path;
use crate::http::{http_post_stream, join_api};
use crate::key::{load_config_if_present, no_key_error, resolve_api_key, resolve_base_url};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::catalog_model_id;
use crate::term;

/// One decoded server-sent-event line.
#[derive(Debug, PartialEq)]
pub(crate) enum Sse {
    Delta(String),
    /// Final chunk with token counts (prompt, completion).
    Usage(u64, u64),
    Done,
    Skip,
}

pub(crate) fn parse_sse_line(line: &str) -> Sse {
    let Some(data) = line.strip_prefix("data:") else {
        return Sse::Skip;
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Sse::Done;
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return Sse::Skip;
    };
    match v
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
    {
        Some(t) if !t.is_empty() => Sse::Delta(t.to_string()),
        _ => match (
            v.pointer("/usage/prompt_tokens").and_then(Value::as_u64),
            v.pointer("/usage/completion_tokens")
                .and_then(Value::as_u64),
        ) {
            (Some(p), Some(c)) => Sse::Usage(p, c),
            _ => Sse::Skip,
        },
    }
}

/// Copy streamed text to `out` as it arrives and return the full reply.
fn stream_reply(reader: impl BufRead, out: &mut impl Write) -> Result<String, String> {
    stream_reply_usage(reader, out).map(|(reply, _)| reply)
}

/// Like `stream_reply`, also returning the `(prompt, completion)` token counts
/// when the server sent a usage chunk.
pub(crate) fn stream_reply_usage(
    reader: impl BufRead,
    out: &mut impl Write,
) -> Result<(String, Option<(u64, u64)>), String> {
    let mut reply = String::new();
    let mut usage = None;
    for line in reader.lines() {
        let line = line.map_err(|e| format!("Stream interrupted: {e}"))?;
        match parse_sse_line(&line) {
            Sse::Delta(t) => {
                reply.push_str(&t);
                let _ = write!(out, "{t}");
                let _ = out.flush();
            }
            Sse::Usage(p, c) => usage = Some((p, c)),
            Sse::Done => break,
            Sse::Skip => {}
        }
    }
    Ok((reply, usage))
}

const CHAT_USAGE: &str =
    "Usage: {bin} chat \"<prompt>\" [--json|--no-stream]   (prompt, `-` for stdin, or pipe it in)";

/// The prompt from the words after `chat`. A lone `-` stands for stdin, so
/// `git diff | anyr chat "review this" -` sends both. `None`: no words.
fn read_prompt(words: &[String], stdin: &mut dyn BufRead) -> Result<Option<String>, String> {
    if words.is_empty() {
        return Ok(None);
    }
    let mut parts = Vec::with_capacity(words.len());
    for w in words {
        if w == "-" {
            let mut piped = String::new();
            stdin
                .read_to_string(&mut piped)
                .map_err(|e| format!("Could not read stdin: {e}"))?;
            parts.push(piped.trim_end().to_string());
        } else {
            parts.push(w.clone());
        }
    }
    Ok(Some(parts.join(" ")))
}

/// `--json` output for one non-streamed reply. `id` is the generation id
/// `anyr generation <id>` takes.
pub(crate) fn reply_json(
    headers: &[(String, String)],
    body: &Value,
    fallback_model: &str,
) -> Value {
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    let id = header("x-anyrouter-generation-id")
        .or_else(|| {
            body.pointer("/anyrouter_metadata/requestId")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| body.get("id").and_then(Value::as_str).map(str::to_string));
    json!({
        "id": id,
        "model": body.get("model").and_then(Value::as_str).unwrap_or(fallback_model),
        "content": body.pointer("/choices/0/message/content").and_then(Value::as_str).unwrap_or_default(),
        "finish_reason": body.pointer("/choices/0/finish_reason").cloned().unwrap_or(Value::Null),
        "usage": body.get("usage").cloned().unwrap_or(Value::Null),
    })
}

struct Session {
    url: String,
    key: String,
    model: String,
    messages: Vec<Value>,
}

impl Session {
    /// One non-streamed turn; returns the `reply_json` shape.
    fn ask(&mut self, user: &str) -> Result<Value, String> {
        self.messages.push(json!({"role": "user", "content": user}));
        let body = json!({"model": self.model, "messages": self.messages, "stream": false});
        let (status, headers, text) =
            crate::http::http_post_json_headers(&self.url, &self.key, &body.to_string())?;
        if !(200..300).contains(&status) {
            self.messages.pop();
            return Err(crate::http::status_error(status, &text));
        }
        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("Invalid chat response: {e}"))?;
        Ok(reply_json(&headers, &value, &self.model))
    }

    /// Send one user turn, stream the answer to stdout, remember both sides.
    fn turn(&mut self, user: &str) -> Result<(), String> {
        self.messages.push(json!({"role": "user", "content": user}));
        let body = json!({"model": self.model, "messages": self.messages, "stream": true});
        let (status, mut reader) = http_post_stream(&self.url, &self.key, &body.to_string())?;
        if !(200..300).contains(&status) {
            self.messages.pop();
            let mut text = String::new();
            let _ = reader.read_to_string(&mut text);
            return Err(crate::http::status_error(status, &text));
        }
        let reply = stream_reply(io::BufReader::new(reader), &mut io::stdout().lock())?;
        println!();
        self.messages
            .push(json!({"role": "assistant", "content": reply}));
        Ok(())
    }
}

pub(crate) fn run_chat(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let existing = load_config_if_present(&path);
    let profile = existing
        .as_ref()
        .and_then(|c| c.profiles.get(&c.active_profile));
    let key = resolve_api_key(&parsed.flags, env, profile).ok_or_else(no_key_error)?;
    let base = resolve_base_url(&parsed.flags, profile);
    let model = get_string_flag(&parsed.flags, "model")
        .or_else(|| profile.map(|p| p.default_model().to_string()))
        .unwrap_or_else(|| crate::config::DEFAULT_MODEL.to_string());
    let mut messages = Vec::new();
    if let Some(system) = get_string_flag(&parsed.flags, "system") {
        messages.push(json!({"role": "system", "content": system}));
    }
    let mut session = Session {
        url: join_api(&base, "/v1/chat/completions"),
        key,
        model: catalog_model_id(&model),
        messages,
    };

    let json_out = parsed.flag_true("json");
    let buffered = json_out || parsed.flag_true("no-stream");
    // Bind first: the stdin lock must drop before the arm below reads stdin.
    let words = read_prompt(&parsed.passthrough, &mut io::stdin().lock())?;
    let prompt = match words {
        Some(p) => Some(p),
        None if !term::is_interactive() => {
            let mut piped = String::new();
            io::stdin()
                .read_to_string(&mut piped)
                .map_err(|e| format!("Could not read stdin: {e}"))?;
            Some(piped)
        }
        None => None,
    };
    if let Some(prompt) = prompt {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(crate::cmd::dispatch::hint(CHAT_USAGE));
        }
        if !buffered {
            session.turn(prompt)?;
            return Ok(0);
        }
        let reply = session.ask(prompt)?;
        if json_out {
            println!("{}", reply);
        } else {
            println!("{}", reply["content"].as_str().unwrap_or_default());
        }
        return Ok(0);
    }
    if buffered {
        return Err(crate::cmd::dispatch::hint(CHAT_USAGE));
    }
    eprintln!(
        "{}",
        term::dim(&format!(
            "{}  (type /exit or Ctrl-D to quit)",
            session.model
        ))
    );
    loop {
        eprint!("{} ", term::accent(">"));
        let _ = io::stderr().flush();
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            eprintln!();
            return Ok(0);
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if matches!(line, "/exit" | "/quit" | "exit" | "quit") {
            return Ok(0);
        }
        // A failed turn must not end the session.
        if let Err(e) = session.turn(line) {
            eprintln!("{} {e}", term::err("error:"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dash_word_reads_stdin_so_a_prompt_and_piped_text_combine() {
        let mut stdin = "diff --git a b\n+x\n".as_bytes();
        let words = vec!["review this".to_string(), "-".to_string()];
        let got = read_prompt(&words, &mut stdin).unwrap();
        assert_eq!(got.as_deref(), Some("review this diff --git a b\n+x"));
        // No words: the caller decides (stdin or REPL); stdin is untouched.
        let mut untouched = "keep".as_bytes();
        assert_eq!(read_prompt(&[], &mut untouched).unwrap(), None);
        assert_eq!(untouched, b"keep");
    }

    #[test]
    fn reply_json_prefers_the_generation_header_then_request_id() {
        let body = json!({
            "id": "chatcmpl-1",
            "model": "z-ai/glm",
            "anyrouter_metadata": {"requestId": "req_9"},
            "choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 1}
        });
        let with_header = vec![("x-anyrouter-generation-id".to_string(), "gen_7".to_string())];
        assert_eq!(reply_json(&with_header, &body, "auto")["id"], "gen_7");
        let v = reply_json(&[], &body, "auto");
        assert_eq!(v["id"], "req_9");
        assert_eq!(v["content"], "ok");
        assert_eq!(v["finish_reason"], "stop");
        assert_eq!(v["model"], "z-ai/glm");
        assert_eq!(v["usage"]["completion_tokens"], 1);
    }

    #[test]
    fn sse_lines_decode_deltas_done_and_noise() {
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[{"delta":{"content":"Hi"}}]}"#),
            Sse::Delta("Hi".into())
        );
        assert_eq!(parse_sse_line("data: [DONE]"), Sse::Done);
        assert_eq!(parse_sse_line(": keep-alive"), Sse::Skip);
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#),
            Sse::Skip
        );
    }

    #[test]
    fn stream_reply_writes_each_delta_and_stops_at_done() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
                   data: [DONE]\n\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n";
        let mut out = Vec::new();
        let reply = stream_reply(sse.as_bytes(), &mut out).unwrap();
        assert_eq!(reply, "Hello");
        assert_eq!(String::from_utf8(out).unwrap(), "Hello");
    }
}
