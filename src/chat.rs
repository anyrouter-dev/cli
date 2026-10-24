//! `anyr chat [prompt]` — minimal streaming chat over `POST /v1/chat/completions`.
//! One-shot with a prompt argument (or piped stdin); a REPL on a terminal.

use std::collections::BTreeMap;
use std::io::{self, BufRead, Read, Write};

use serde_json::{json, Value};

use crate::cmd::dispatch::config_path;
use crate::http::{error_message, http_post_stream, join_api};
use crate::key::{load_config_if_present, no_key_error, resolve_api_key, resolve_base_url};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::catalog_model_id;
use crate::term;

/// One decoded server-sent-event line.
#[derive(Debug, PartialEq)]
enum Sse {
    Delta(String),
    Done,
    Skip,
}

fn parse_sse_line(line: &str) -> Sse {
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
        _ => Sse::Skip,
    }
}

/// Copy streamed text to `out` as it arrives and return the full reply.
fn stream_reply(reader: impl BufRead, out: &mut impl Write) -> Result<String, String> {
    let mut reply = String::new();
    for line in reader.lines() {
        let line = line.map_err(|e| format!("Stream interrupted: {e}"))?;
        match parse_sse_line(&line) {
            Sse::Delta(t) => {
                reply.push_str(&t);
                let _ = write!(out, "{t}");
                let _ = out.flush();
            }
            Sse::Done => break,
            Sse::Skip => {}
        }
    }
    Ok(reply)
}

struct Session {
    url: String,
    key: String,
    model: String,
    messages: Vec<Value>,
}

impl Session {
    /// Send one user turn, stream the answer to stdout, remember both sides.
    fn turn(&mut self, user: &str) -> Result<(), String> {
        self.messages.push(json!({"role": "user", "content": user}));
        let body = json!({"model": self.model, "messages": self.messages, "stream": true});
        let (status, mut reader) = http_post_stream(&self.url, &self.key, &body.to_string())?;
        if !(200..300).contains(&status) {
            self.messages.pop();
            let mut text = String::new();
            let _ = reader.read_to_string(&mut text);
            return Err(error_message(status, &text));
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

    let prompt = parsed.passthrough.join(" ");
    if !prompt.trim().is_empty() {
        session.turn(prompt.trim())?;
        return Ok(0);
    }
    if !term::is_interactive() {
        let mut piped = String::new();
        io::stdin()
            .read_to_string(&mut piped)
            .map_err(|e| format!("Could not read stdin: {e}"))?;
        if piped.trim().is_empty() {
            return Err(crate::cmd::dispatch::hint(
                "Usage: {bin} chat \"<prompt>\"   (or run it in a terminal for a REPL)",
            ));
        }
        session.turn(piped.trim())?;
        return Ok(0);
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
