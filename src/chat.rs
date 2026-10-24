//! `anyr chat [prompt]` — minimal streaming chat over `POST /v1/chat/completions`.
//! One-shot with a prompt argument (or piped stdin); a REPL on a terminal.

use std::collections::BTreeMap;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::cmd::dispatch::config_path;
use crate::http::{http_post_stream_headers, join_api};
use crate::key::{load_config_if_present, no_key_error, resolve_api_key, resolve_base_url};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::catalog_model_id;
use crate::term;

/// What the gateway reported on one stream chunk. Fields the chunk did not
/// carry stay `None`; chunks are merged with `absorb`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct ChunkMeta {
    model: Option<String>,
    /// `anyrouter_metadata.requestId`.
    request_id: Option<String>,
    /// The upstream `id` of the chunk (`gen-…`), the last resort for an id.
    chunk_id: Option<String>,
    usage: Option<(u64, u64)>,
    cost: Option<f64>,
}

impl ChunkMeta {
    fn absorb(&mut self, other: ChunkMeta) {
        self.model = other.model.or(self.model.take());
        self.request_id = other.request_id.or(self.request_id.take());
        self.chunk_id = other.chunk_id.or(self.chunk_id.take());
        self.usage = other.usage.or(self.usage);
        self.cost = other.cost.or(self.cost);
    }
}

/// One decoded server-sent-event line.
#[derive(Debug, PartialEq)]
pub(crate) enum Sse {
    Chunk {
        delta: Option<String>,
        meta: ChunkMeta,
    },
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
    let text = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_string);
    let tokens = |p: &str| v.pointer(p).and_then(Value::as_u64);
    let delta = text("/choices/0/delta/content").filter(|t| !t.is_empty());
    let usage = match (
        tokens("/usage/prompt_tokens"),
        tokens("/usage/completion_tokens"),
    ) {
        (Some(p), Some(c)) => Some((p, c)),
        _ => match (
            tokens("/anyrouter_metadata/usage/inputTokens"),
            tokens("/anyrouter_metadata/usage/outputTokens"),
        ) {
            (Some(p), Some(c)) => Some((p, c)),
            _ => None,
        },
    };
    Sse::Chunk {
        delta,
        meta: ChunkMeta {
            model: text("/model"),
            request_id: text("/anyrouter_metadata/requestId"),
            chunk_id: text("/id"),
            usage,
            cost: v.pointer("/usage/cost").and_then(Value::as_f64),
        },
    }
}

/// Something that happened on the wire, in order.
enum Event {
    Delta(String),
    Meta(ChunkMeta),
    End,
    Failed(String),
}

/// Decode `reader` and hand each event to `on`. `on` returns `false` to stop.
fn read_events(reader: impl BufRead, mut on: impl FnMut(Event) -> bool) {
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                on(Event::Failed(format!("Stream interrupted: {e}")));
                return;
            }
        };
        let keep_going = match parse_sse_line(&line) {
            Sse::Chunk { delta, meta } => {
                delta.is_none_or(|t| on(Event::Delta(t))) && on(Event::Meta(meta))
            }
            Sse::Done => {
                on(Event::End);
                return;
            }
            Sse::Skip => true,
        };
        if !keep_going {
            return;
        }
    }
    on(Event::End);
}

/// Copy streamed text to `out` as it arrives and return the full reply.
#[cfg(test)]
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
    let mut meta = ChunkMeta::default();
    let mut failure = None;
    read_events(reader, |ev| {
        match ev {
            Event::Delta(t) => {
                reply.push_str(&t);
                let _ = write!(out, "{t}");
                let _ = out.flush();
            }
            Event::Meta(m) => meta.absorb(m),
            Event::End => {}
            Event::Failed(e) => failure = Some(e),
        }
        failure.is_none()
    });
    match failure {
        Some(e) => Err(e),
        None => Ok((reply, meta.usage)),
    }
}

/// Facts about the latest reply, filled from the stream. A field the server
/// did not send stays `None` and is left out of the footer.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct TurnStats {
    pub model: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    /// Request sent to first text.
    pub ttft: Option<Duration>,
    pub total: Duration,
    /// USD, as the gateway reported it.
    pub cost: Option<f64>,
    pub request_id: Option<String>,
}

impl TurnStats {
    fn new(
        headers: &[(String, String)],
        meta: ChunkMeta,
        ttft: Option<Duration>,
        total: Duration,
    ) -> Self {
        let header = |name: &str| {
            headers
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        TurnStats {
            model: meta.model.or_else(|| header("x-anyrouter-model")),
            prompt_tokens: meta.usage.map(|u| u.0),
            completion_tokens: meta.usage.map(|u| u.1),
            ttft,
            total,
            cost: meta.cost,
            request_id: header("x-anyrouter-generation-id")
                .or(meta.request_id)
                .or_else(|| header("x-request-id"))
                .or(meta.chunk_id),
        }
    }

    /// The one dim line shown under the newest reply.
    pub fn footer(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.extend(self.model.clone());
        if let (Some(p), Some(c)) = (self.prompt_tokens, self.completion_tokens) {
            parts.push(format!("{p}→{c} tokens"));
        }
        if let Some(t) = self.ttft {
            parts.push(format!("first token {}", fmt_dur(t)));
        }
        parts.push(fmt_dur(self.total));
        let generating = self.total.saturating_sub(self.ttft.unwrap_or_default());
        if let Some(c) = self.completion_tokens.filter(|c| *c > 0) {
            if generating >= Duration::from_millis(50) {
                parts.push(format!("{:.0} tok/s", c as f64 / generating.as_secs_f64()));
            }
        }
        if let Some(cost) = self.cost {
            parts.push(if cost > 0.0 {
                crate::http::format_usd(cost)
            } else {
                "free".to_string()
            });
        }
        if let Some(id) = &self.request_id {
            parts.push(id.chars().take(12).collect());
        }
        parts.join(" · ")
    }

    /// `timing` for the `--json` object. Only what was measured.
    fn timing_json(&self) -> Value {
        let mut t = serde_json::Map::new();
        if let Some(d) = self.ttft {
            t.insert("ttft_ms".into(), json!(d.as_millis() as u64));
        }
        t.insert("total_ms".into(), json!(self.total.as_millis() as u64));
        Value::Object(t)
    }
}

fn fmt_dur(d: Duration) -> String {
    if d < Duration::from_secs(1) {
        format!("{}ms", d.as_millis())
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

static CANCEL: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigint(_: libc::c_int) {
    CANCEL.store(true, Ordering::SeqCst);
}

/// While alive, Ctrl-C sets `CANCEL` instead of killing the process, so a
/// reply can be stopped and the session kept. Dropping puts back whatever
/// handler was there before (a shell that started us with SIGINT ignored
/// keeps it ignored).
struct CancelOnCtrlC {
    /// `None` when installing the handler failed, so there is nothing to undo.
    prev: Option<libc::sighandler_t>,
}

impl CancelOnCtrlC {
    fn arm() -> Self {
        CANCEL.store(false, Ordering::SeqCst);
        let prev =
            unsafe { libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t) };
        CancelOnCtrlC {
            // SIG_ERR is all ones; its libc type differs between unix and windows.
            prev: (prev != libc::sighandler_t::MAX).then_some(prev),
        }
    }
}

impl Drop for CancelOnCtrlC {
    fn drop(&mut self) {
        if let Some(prev) = self.prev {
            unsafe { libc::signal(libc::SIGINT, prev) };
        }
    }
}

/// How one streamed reply ended.
struct Streamed {
    reply: String,
    meta: ChunkMeta,
    ttft: Option<Duration>,
    cancelled: bool,
}

/// Print deltas as they arrive. The wire is read on a helper thread so the
/// caller can notice `cancel` between chunks; a cancelled read thread exits
/// (and drops the connection) when its next send finds nobody listening.
fn stream_live(
    reader: impl BufRead + Send + 'static,
    out: &mut impl Write,
    start: Instant,
    cancel: &AtomicBool,
) -> Result<Streamed, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || read_events(reader, |ev| tx.send(ev).is_ok()));
    let mut s = Streamed {
        reply: String::new(),
        meta: ChunkMeta::default(),
        ttft: None,
        cancelled: false,
    };
    loop {
        if cancel.load(Ordering::SeqCst) {
            s.cancelled = true;
            return Ok(s);
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Delta(t)) => {
                s.ttft.get_or_insert_with(|| start.elapsed());
                s.reply.push_str(&t);
                let _ = write!(out, "{t}");
                let _ = out.flush();
            }
            Ok(Event::Meta(m)) => s.meta.absorb(m),
            Ok(Event::Failed(e)) => return Err(e),
            Ok(Event::End) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(s),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
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
    /// Print the stats line under each reply (a terminal, not machine output).
    footer: bool,
    /// Ctrl-C stops the reply and keeps the session (the REPL).
    cancellable: bool,
}

impl Session {
    /// One non-streamed turn; returns the `reply_json` shape plus `timing`.
    fn ask(&mut self, user: &str) -> Result<Value, String> {
        self.messages.push(json!({"role": "user", "content": user}));
        let body = json!({"model": self.model, "messages": self.messages, "stream": false});
        let start = Instant::now();
        let (status, headers, text) =
            crate::http::http_post_json_headers(&self.url, &self.key, &body.to_string())?;
        let total = start.elapsed();
        if !(200..300).contains(&status) {
            self.messages.pop();
            return Err(crate::http::status_error(status, &text));
        }
        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("Invalid chat response: {e}"))?;
        let mut reply = reply_json(&headers, &value, &self.model);
        reply["timing"] = TurnStats {
            total,
            ..TurnStats::default()
        }
        .timing_json();
        Ok(reply)
    }

    /// Send one user turn, stream the answer to stdout, remember both sides.
    /// `Ok(Some(mark))` when a footer was printed under the reply and can be
    /// erased later (see `erase_rows`).
    fn turn(&mut self, user: &str) -> Result<Option<FooterMark>, String> {
        self.messages.push(json!({"role": "user", "content": user}));
        let body = json!({
            "model": self.model,
            "messages": self.messages,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        let _guard = self.cancellable.then(CancelOnCtrlC::arm);
        let start = Instant::now();
        let (status, headers, mut reader) =
            match http_post_stream_headers(&self.url, &self.key, &body.to_string()) {
                Ok(r) => r,
                Err(e) => {
                    self.messages.pop();
                    return Err(e);
                }
            };
        if !(200..300).contains(&status) {
            self.messages.pop();
            let mut text = String::new();
            let _ = reader.read_to_string(&mut text);
            return Err(crate::http::status_error(status, &text));
        }
        let streamed = stream_live(
            io::BufReader::new(reader),
            &mut io::stdout().lock(),
            start,
            &CANCEL,
        );
        let streamed = match streamed {
            Ok(s) => s,
            Err(e) => {
                println!();
                self.messages.pop();
                return Err(e);
            }
        };
        println!();
        if streamed.cancelled {
            self.messages.pop();
            println!("{}", term::dim("(stopped)"));
            return Ok(None);
        }
        self.messages
            .push(json!({"role": "assistant", "content": streamed.reply}));
        if !self.footer {
            return Ok(None);
        }
        let stats = TurnStats::new(&headers, streamed.meta, streamed.ttft, start.elapsed());
        Ok(print_footer(&stats.footer()))
    }
}

/// Terminal size (columns, rows) when a footer was printed.
type FooterMark = (u16, u16);

fn terminal_size() -> Option<FooterMark> {
    crossterm::terminal::size().ok().filter(|(c, _)| *c > 0)
}

/// Display width of `c`; anything outside ASCII (bar the two footer
/// separators) counts double, so a line is never cut too late.
fn char_width(c: char) -> usize {
    if c.is_ascii() || matches!(c, '→' | '·') {
        1
    } else {
        2
    }
}

/// `line` without control characters, cut to at most `cols - 1` columns so it
/// can never wrap onto a second row.
fn fit_to_width(line: &str, cols: usize) -> String {
    let max = cols.saturating_sub(1);
    let mut used = 0;
    let mut out = String::new();
    for c in line.chars().filter(|c| !c.is_control()) {
        used += char_width(c);
        if used > max {
            break;
        }
        out.push(c);
    }
    out
}

/// Print the dim footer line. Returns the terminal size to erase it by, or
/// `None` when the size is unknown (the line is then left alone for good).
fn print_footer(footer: &str) -> Option<FooterMark> {
    let size = terminal_size();
    let line = match size {
        Some((cols, _)) => fit_to_width(footer, usize::from(cols)),
        None => footer.chars().filter(|c| !c.is_control()).collect(),
    };
    println!("{}", term::dim(&line));
    size
}

/// True when stdin already holds input (or cannot be asked). That input was
/// typed while the reply streamed and its echo is already on screen, so the
/// screen no longer matches what the erase assumes. Never flushes it.
#[cfg(unix)]
fn stdin_has_pending() -> bool {
    let mut fd = libc::pollfd {
        fd: 0,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut fd, 1, 0) != 0 }
}

#[cfg(not(unix))]
fn stdin_has_pending() -> bool {
    true
}

/// How many input rows to step over to erase the footer above the prompt the
/// user just submitted, or `None` when erasing is not certain. A stale dim
/// line is fine; deleting reply text or typing never is.
fn erase_rows(
    mark: FooterMark,
    now: Option<FooterMark>,
    submitted: &str,
    pending: bool,
) -> Option<usize> {
    let printable = submitted.bytes().all(|b| (0x20..0x7f).contains(&b));
    if pending || now != Some(mark) || !printable {
        return None;
    }
    // "> " prompt plus the echoed input, wrapped at the terminal width.
    Some((2 + submitted.len()).div_ceil(usize::from(mark.0)).max(1))
}

/// The cursor sits below the echoed input (`rows` lines); the footer is the
/// line right above that.
fn erase_footer(rows: usize) {
    print!("\x1b[{}A\r\x1b[M\x1b[{}B", rows + 1, rows);
    let _ = io::stdout().flush();
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
        footer: io::stdout().is_terminal()
            && io::stderr().is_terminal()
            && !term::machine_output(parsed, env),
        cancellable: false,
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
    session.cancellable = true;
    let mut footer_up: Option<FooterMark> = None;
    loop {
        let pending = footer_up.is_some() && stdin_has_pending();
        eprint!("{} ", term::accent(">"));
        let _ = io::stderr().flush();
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            eprintln!();
            return Ok(0);
        }
        if let Some(mark) = footer_up.take() {
            let submitted = line.trim_end_matches(['\r', '\n']);
            if let Some(rows) = erase_rows(mark, terminal_size(), submitted, pending) {
                erase_footer(rows);
            }
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if matches!(line, "/exit" | "/quit" | "exit" | "quit") {
            return Ok(0);
        }
        // A failed turn must not end the session.
        match session.turn(line) {
            Ok(mark) => footer_up = mark,
            Err(e) => eprintln!(
                "{} {}",
                term::err("error:"),
                crate::key::mask_secrets(&e.to_string())
            ),
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
        let Sse::Chunk { delta, meta } = parse_sse_line(
            r#"data: {"id":"g1","model":"m","choices":[{"delta":{"content":"Hi"}}]}"#,
        ) else {
            panic!("expected a chunk");
        };
        assert_eq!(delta.as_deref(), Some("Hi"));
        assert_eq!(meta.model.as_deref(), Some("m"));
        assert_eq!(meta.chunk_id.as_deref(), Some("g1"));
        assert_eq!(parse_sse_line("data: [DONE]"), Sse::Done);
        assert_eq!(parse_sse_line(": keep-alive"), Sse::Skip);
        let Sse::Chunk { delta, .. } =
            parse_sse_line(r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#)
        else {
            panic!("expected a chunk");
        };
        assert_eq!(delta, None);
    }

    const FINAL: &str = r#"data: {"id":"gen-1","model":"cohere/x","choices":[{"delta":{"content":""}}],"usage":{"prompt_tokens":2,"completion_tokens":36,"cost":0.0012}}"#;
    const META: &str = r#"data: {"id":"req_9725c9bdc61c4a74","model":"cohere/x","choices":[],"anyrouter_metadata":{"requestId":"req_9725c9bdc61c4a74"}}"#;

    #[test]
    fn stats_come_from_the_final_usage_and_metadata_chunks() {
        let mut meta = ChunkMeta::default();
        for l in [FINAL, META] {
            let Sse::Chunk { meta: m, .. } = parse_sse_line(l) else {
                panic!("chunk")
            };
            meta.absorb(m);
        }
        let stats = TurnStats::new(
            &[],
            meta,
            Some(Duration::from_millis(250)),
            Duration::from_millis(1400),
        );
        assert_eq!(
            stats.footer(),
            "cohere/x · 2→36 tokens · first token 250ms · 1.4s · 31 tok/s · $0.0012 · req_9725c9bd"
        );
    }

    #[test]
    fn footer_leaves_out_what_the_server_did_not_send() {
        let stats = TurnStats {
            total: Duration::from_millis(800),
            ..TurnStats::default()
        };
        assert_eq!(stats.footer(), "800ms");
        let free = TurnStats {
            cost: Some(0.0),
            total: Duration::from_millis(800),
            ..TurnStats::default()
        };
        assert_eq!(free.footer(), "800ms · free");
    }

    #[test]
    fn the_generation_header_wins_the_request_id() {
        let headers = vec![
            ("x-request-id".to_string(), "req_h".to_string()),
            ("x-anyrouter-generation-id".to_string(), "gen_h".to_string()),
        ];
        let meta = ChunkMeta {
            request_id: Some("req_m".into()),
            ..ChunkMeta::default()
        };
        let s = TurnStats::new(&headers, meta, None, Duration::ZERO);
        assert_eq!(s.request_id.as_deref(), Some("gen_h"));
    }

    /// A reader that yields one SSE line per `read` call, so the stream is
    /// consumed in separate steps like a real socket.
    struct Trickle(std::collections::VecDeque<Vec<u8>>);
    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.0.pop_front() else {
                return Ok(0);
            };
            buf[..chunk.len()].copy_from_slice(&chunk);
            Ok(chunk.len())
        }
    }

    /// Records the size of every `write` call.
    #[derive(Default)]
    struct Writes(Vec<usize>);
    impl Write for Writes {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.push(b.len());
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn trickle(lines: &[&str]) -> io::BufReader<Trickle> {
        let parts = lines.iter().map(|l| format!("{l}\n\n").into_bytes());
        io::BufReader::new(Trickle(parts.collect()))
    }

    #[test]
    fn live_stream_writes_each_delta_separately_and_reports_ttft() {
        let lines = [
            r#"data: {"choices":[{"delta":{"content":"a"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"bc"}}]}"#,
            FINAL,
            "data: [DONE]",
        ];
        let mut out = Writes::default();
        let s = stream_live(
            trickle(&lines),
            &mut out,
            Instant::now(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(s.reply, "abc");
        assert_eq!(
            out.0,
            vec![1, 2],
            "one write per delta, before the stream ended"
        );
        assert!(s.ttft.is_some());
        assert_eq!(s.meta.usage, Some((2, 36)));
        assert!(!s.cancelled);
    }

    #[test]
    fn the_footer_is_cut_to_one_row_and_stripped_of_control_chars() {
        let cut = fit_to_width("cohere/x · 7→11 tokens · 1.4s", 12);
        assert_eq!(cut, "cohere/x · ");
        assert!(cut.chars().count() < 12, "leave the last column free");
        // WHY: a model name comes from the server; an escape must not run.
        assert_eq!(fit_to_width("a\x1b[2Jb", 80), "a[2Jb");
        // Wide characters count double, so the line stops early, never late.
        assert_eq!(fit_to_width("日本語日本語", 8), "日本語");
    }

    #[test]
    fn erase_only_when_the_screen_is_the_one_we_drew() {
        let mark = (80, 24);
        fn ok(now: Option<FooterMark>, line: &str, pending: bool) -> Option<usize> {
            erase_rows((80, 24), now, line, pending)
        }
        assert_eq!(ok(Some(mark), "hello", false), Some(1));
        // 2 + 100 columns of echo wrap onto a second row.
        assert_eq!(ok(Some(mark), &"a".repeat(100), false), Some(2));
        // Typed ahead: its echo is already on screen.
        assert_eq!(ok(Some(mark), "hello", true), None);
        // Resized (reflow moved the footer) or size unknown.
        assert_eq!(ok(Some((60, 24)), "hello", false), None);
        assert_eq!(ok(Some((80, 30)), "hello", false), None);
        assert_eq!(ok(None, "hello", false), None);
        // Non-ASCII or control input may wrap unpredictably.
        assert_eq!(ok(Some(mark), "héllo", false), None);
        assert_eq!(ok(Some(mark), "a\tb", false), None);
    }

    #[cfg(unix)]
    #[test]
    fn ctrl_c_handler_is_put_back_as_it_was() {
        let sig = |h| unsafe { libc::signal(libc::SIGINT, h) };
        // A parent that ignores SIGINT must still be ignoring it afterwards.
        sig(libc::SIG_IGN);
        drop(CancelOnCtrlC::arm());
        assert_eq!(sig(libc::SIG_DFL), libc::SIG_IGN);
        // And a default disposition comes back as the default.
        drop(CancelOnCtrlC::arm());
        assert_eq!(sig(libc::SIG_DFL), libc::SIG_DFL);
    }

    #[test]
    fn a_set_cancel_flag_stops_the_reply_without_an_error() {
        struct Hang;
        impl Read for Hang {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                std::thread::sleep(Duration::from_secs(5));
                Ok(0)
            }
        }
        let cancel = AtomicBool::new(true);
        let t = Instant::now();
        let s = stream_live(io::BufReader::new(Hang), &mut Writes::default(), t, &cancel).unwrap();
        assert!(s.cancelled);
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "must not wait on the socket"
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
