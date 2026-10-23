//! `anyr embed`: embeddings from the terminal (`POST /v1/embeddings`).

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::api::{context, get, read_file_capped, read_stdin, rows, send};
use crate::cmd::dispatch::{hint, USAGE};
use crate::cmd::view::print_json;
use crate::http::format_usd;
use crate::parse::{get_string_flag, ParsedArgs};
use crate::term;

const USAGE_LINE: &str =
    "Usage: {bin} embed [--model <id>] [--dimensions N] [--file <path>] [--json] <text...|->";
const PREVIEW: usize = 8;

/// One input per non-empty line.
fn lines(text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
}

/// Gather inputs: words as given, `-` reads stdin lines, `--file` adds file lines.
pub(crate) fn collect_inputs(
    words: &[String],
    file: Option<&str>,
    mut stdin: impl FnMut() -> Result<String, String>,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for w in words {
        if w == "-" {
            out.extend(lines(&stdin()?));
        } else {
            out.push(w.clone());
        }
    }
    if let Some(path) = file {
        out.extend(lines(&read_file_capped(path)?));
    }
    if out.is_empty() {
        return Err(hint(USAGE_LINE));
    }
    Ok(out)
}

pub(crate) fn request_body(model: &str, inputs: &[String], dimensions: Option<u32>) -> Value {
    let mut body = json!({"model": model, "input": inputs});
    if let Some(d) = dimensions {
        body["dimensions"] = json!(d);
    }
    body
}

/// First embedding model, preferring ones not billed to the caller's own key.
pub(crate) fn default_embedding_model(catalog: &Value) -> Option<String> {
    let all = rows(catalog);
    let models: Vec<&Value> = all
        .iter()
        .filter(|m| m.get("category").and_then(Value::as_str) == Some("embedding"))
        .collect();
    let byok = |m: &&&Value| m.pointer("/pricing/billing").and_then(Value::as_str) == Some("byok");
    models
        .iter()
        .find(|m| !byok(m))
        .or_else(|| models.first())
        .and_then(|m| m.get("id").and_then(Value::as_str))
        .map(str::to_string)
}

fn preview(vector: &[Value]) -> String {
    let head: Vec<String> = vector
        .iter()
        .take(PREVIEW)
        .map(|v| format!("{:.4}", v.as_f64().unwrap_or(0.0)))
        .collect();
    let more = if vector.len() > PREVIEW { ", …" } else { "" };
    format!("[{}{more}]", head.join(", "))
}

pub(crate) fn render(resp: &Value, inputs: &[String]) -> String {
    let data = resp.get("data").and_then(Value::as_array);
    let dims = data
        .and_then(|d| d.first())
        .and_then(|d| d.get("embedding"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let mut out = format!(
        "{} {}\n{} {}\n{} {}\n",
        term::dim("model     "),
        resp.get("model").and_then(Value::as_str).unwrap_or("-"),
        term::dim("inputs    "),
        inputs.len(),
        term::dim("dimensions"),
        dims
    );
    if let Some(t) = resp.pointer("/usage/total_tokens").and_then(Value::as_u64) {
        out += &format!("{} {t}\n", term::dim("tokens    "));
    }
    let cost = resp
        .pointer("/usage/cost")
        .or_else(|| resp.pointer("/usage/cost_usd"))
        .and_then(Value::as_f64);
    if let Some(c) = cost {
        out += &format!("{} {}\n", term::dim("cost      "), format_usd(c));
    }
    for (i, item) in data.into_iter().flatten().enumerate() {
        let vector = item
            .get("embedding")
            .and_then(Value::as_array)
            .map(|v| preview(v))
            .unwrap_or_else(|| "-".into());
        let label: String = inputs
            .get(i)
            .map(|s| s.chars().take(40).collect())
            .unwrap_or_default();
        out += &format!("\n{} {label}\n  {vector}\n", term::dim(&format!("#{i}")));
    }
    out
}

pub(crate) fn run_embed(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let dimensions = match get_string_flag(&parsed.flags, "dimensions") {
        None => None,
        Some(raw) => match raw.parse::<u32>() {
            Ok(n) if n > 0 => Some(n),
            _ => return Err(format!("{USAGE}--dimensions must be a positive number.")),
        },
    };
    let inputs = collect_inputs(
        &parsed.passthrough,
        get_string_flag(&parsed.flags, "file").as_deref(),
        read_stdin,
    )?;
    let ctx = context(parsed, env);
    ctx.key()?;
    let model = match get_string_flag(&parsed.flags, "model") {
        Some(m) => m,
        None => default_embedding_model(&get(&ctx, "/v1/models", false)?)
            .ok_or("no embedding model in the catalog; pass --model <id>".to_string())?,
    };
    let body = request_body(&model, &inputs, dimensions).to_string();
    let resp = send(&ctx, "POST", "/v1/embeddings", Some(&body), true)?;
    if ctx.json {
        print_json(&resp);
    } else {
        print!("{}", render(&resp, &inputs));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_from_args_stdin_and_blank_lines() {
        let words = vec!["a".to_string(), "-".to_string(), "b".to_string()];
        let got = collect_inputs(&words, None, || Ok("x\n\n y \n".into())).unwrap();
        assert_eq!(got, ["a", "x", "y", "b"]);
    }

    #[test]
    fn no_input_is_a_usage_error() {
        let err = collect_inputs(&[], None, || Ok(String::new())).unwrap_err();
        assert!(err.starts_with(USAGE), "{err}");
        let err = collect_inputs(&["-".into()], None, || Ok("\n".into())).unwrap_err();
        assert!(err.starts_with(USAGE), "{err}");
    }

    #[test]
    fn many_inputs_batch_into_one_input_array() {
        // WHY: one request for N texts, not N requests; dimensions only when asked.
        let b = request_body("m", &["a".into(), "b".into()], None);
        assert_eq!(b, json!({"model": "m", "input": ["a", "b"]}));
        let b = request_body("m", &["a".into()], Some(256));
        assert_eq!(b["dimensions"], 256);
    }

    #[test]
    fn default_model_skips_byok_and_non_embedding() {
        let cat = json!({"data": [
            {"id": "t/chat", "category": "text"},
            {"id": "e/byok", "category": "embedding", "pricing": {"billing": "byok"}},
            {"id": "e/ok", "category": "embedding", "pricing": {}}
        ]});
        assert_eq!(default_embedding_model(&cat).as_deref(), Some("e/ok"));
        let only = json!({"data": [{"id": "e/byok", "category": "embedding", "pricing": {"billing": "byok"}}]});
        assert_eq!(default_embedding_model(&only).as_deref(), Some("e/byok"));
        assert_eq!(default_embedding_model(&json!({"data": []})), None);
    }

    #[test]
    fn render_truncates_vectors_and_shows_dims() {
        let vec: Vec<f64> = (0..12).map(|i| i as f64 / 10.0).collect();
        let resp =
            json!({"model": "m", "data": [{"embedding": vec}], "usage": {"total_tokens": 3}});
        let out = render(&resp, &["hello".into()]);
        assert!(out.contains("12"), "{out}");
        assert!(out.contains("0.7000, …]"), "{out}");
        assert!(!out.contains("0.8000"), "{out}");
    }
}
