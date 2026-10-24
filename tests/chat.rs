//! `anyr chat` for scripts: `--json`, `--no-stream`, stdin, exit codes.

mod common;
use common::{run, serve};
use serde_json::Value;

const COMPLETION: &str = r#"{"id":"chatcmpl-1","model":"z-ai/glm-4.7-flash",
 "anyrouter_metadata":{"requestId":"req_abc"},
 "choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
 "usage":{"prompt_tokens":5,"completion_tokens":1}}"#;

#[test]
fn json_sends_one_non_stream_request_and_prints_one_object() {
    let (base, rx) = serve(vec![(200, COMPLETION.into())]);
    let (code, out, err) = run(
        &base,
        &[
            "chat",
            "Reply with ok",
            "--model",
            "z-ai/glm-4.7-flash",
            "--json",
        ],
        None,
    );
    assert_eq!(code, 0, "{err}");
    let seen = rx.recv().unwrap();
    assert_eq!(seen.path, "/api/v1/chat/completions");
    let body: Value = serde_json::from_str(&seen.body).unwrap();
    assert_eq!(body["stream"], false);
    assert_eq!(body["messages"][0]["content"], "Reply with ok");
    let v: Value = serde_json::from_str(&out).expect("stdout is one JSON object");
    // WHY: `id` is what `anyr generation <id>` takes, so a script can chain them.
    assert_eq!(v["id"], "req_abc");
    assert_eq!(v["content"], "ok");
    assert_eq!(v["finish_reason"], "stop");
    assert_eq!(v["usage"]["completion_tokens"], 1);
}

#[test]
fn no_stream_prints_only_the_answer() {
    let (base, rx) = serve(vec![(200, COMPLETION.into())]);
    let (code, out, _) = run(&base, &["chat", "hi", "--no-stream"], None);
    assert_eq!(code, 0);
    assert_eq!(out, "ok\n");
    let body: Value = serde_json::from_str(&rx.recv().unwrap().body).unwrap();
    assert_eq!(body["stream"], false);
}

#[test]
fn a_dash_appends_piped_stdin_to_the_prompt() {
    let (base, rx) = serve(vec![(200, COMPLETION.into())]);
    let (code, _, err) = run(
        &base,
        &["chat", "Review this", "-", "--json"],
        Some("+ added line\n"),
    );
    assert_eq!(code, 0, "{err}");
    let body: Value = serde_json::from_str(&rx.recv().unwrap().body).unwrap();
    assert_eq!(body["messages"][0]["content"], "Review this + added line");
}

#[test]
fn piped_stdin_alone_is_the_prompt() {
    let (base, rx) = serve(vec![(200, COMPLETION.into())]);
    let (code, _, err) = run(&base, &["chat", "--json"], Some("from stdin\n"));
    assert_eq!(code, 0, "{err}");
    let body: Value = serde_json::from_str(&rx.recv().unwrap().body).unwrap();
    assert_eq!(body["messages"][0]["content"], "from stdin");
}

#[test]
fn failures_map_to_exit_codes_and_json_errors() {
    for (status, code, name) in [
        (401, 4, "auth"),
        (404, 3, "not_found"),
        (429, 5, "rate_limited"),
    ] {
        let (base, _rx) = serve(vec![(status, r#"{"error":{"message":"nope"}}"#.into())]);
        let (got, out, err) = run(&base, &["chat", "hi", "--json"], None);
        assert_eq!(got, code, "HTTP {status}: {err}");
        assert!(out.is_empty(), "{out}");
        let v: Value = serde_json::from_str(err.trim()).unwrap();
        assert_eq!(v["error"]["code"], name);
        assert_eq!(v["error"]["exit"], code);
        assert!(v["error"]["message"].as_str().unwrap().contains("nope"));
    }
}

#[test]
fn empty_input_is_a_usage_error_and_sends_nothing() {
    let (base, rx) = serve(vec![(200, COMPLETION.into())]);
    let (code, _, err) = run(&base, &["chat", "--json"], None);
    assert_eq!(code, 2, "{err}");
    assert!(rx
        .recv_timeout(std::time::Duration::from_millis(300))
        .is_err());
}
