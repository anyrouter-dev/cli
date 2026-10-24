//! `anyr task` against a local scripted server.

mod common;
use common::{run, serve};
use std::time::Duration;

const PRICE: &str = r#"{"id":"m","pricing":{"input_per_1m":1.0,"output_per_1m":2.0}}"#;

fn ok(text: &str) -> String {
    format!(
        r#"{{"choices":[{{"message":{{"content":"{text}"}}}}],"usage":{{"prompt_tokens":1000000,"completion_tokens":500000}}}}"#
    )
}

fn sse(text: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}}}]}}\n\n\
         data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":5}}}}\n\n\
         data: [DONE]\n\n"
    )
}

#[test]
fn plan_runs_first_and_its_text_reaches_the_do_prompt() {
    let (base, rx) = serve(vec![
        (200, PRICE.into()),
        (200, PRICE.into()),
        (200, ok("1. PLANTEXT")),
        (200, ok("RESULTTEXT")),
    ]);
    let (code, out, err) = run(
        &base,
        &[
            "task",
            "dedupe files",
            "--plan-model",
            "m/p",
            "--do-model",
            "m/d",
            "--yes",
            "--json",
        ],
        None,
    );
    assert_eq!(code, 0, "{out}{err}");
    let seen: Vec<_> = (0..4)
        .map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap())
        .collect();
    let plan: serde_json::Value = serde_json::from_str(&seen[2].body).unwrap();
    let done: serde_json::Value = serde_json::from_str(&seen[3].body).unwrap();
    assert_eq!(seen[2].path, "/api/v1/chat/completions");
    assert_eq!(plan["model"], "m/p");
    assert_eq!(done["model"], "m/d");
    assert!(plan["messages"].to_string().contains("dedupe files"));
    assert!(!plan["messages"].to_string().contains("PLANTEXT"));
    assert!(done["messages"].to_string().contains("1. PLANTEXT"));
    assert!(done["messages"].to_string().contains("dedupe files"));
}

#[test]
fn json_shape_has_plan_result_usage_and_cost() {
    let (base, _rx) = serve(vec![
        (200, PRICE.into()),
        (200, PRICE.into()),
        (200, ok("the plan")),
        (200, ok("the result")),
    ]);
    let (code, out, err) = run(&base, &["task", "x", "--yes", "--json"], None);
    assert_eq!(code, 0, "{out}{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["plan"], "the plan");
    assert_eq!(v["result"], "the result");
    assert_eq!(v["usage"]["plan"]["prompt_tokens"], 1_000_000);
    assert_eq!(v["usage"]["implement"]["completion_tokens"], 500_000);
    // two phases of (1M * $1 + 0.5M * $2)
    assert!((v["cost"].as_f64().unwrap() - 4.0).abs() < 1e-9, "{out}");
}

#[test]
fn streaming_prints_both_answers() {
    let (base, rx) = serve(vec![
        (200, PRICE.into()),
        (200, PRICE.into()),
        (200, sse("PLANSTREAM")),
        (200, sse("DOSTREAM")),
    ]);
    let (code, out, err) = run(&base, &["task", "x", "--yes"], None);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("PLANSTREAM") && out.contains("DOSTREAM"),
        "{out}"
    );
    let seen: Vec<_> = (0..4)
        .map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap())
        .collect();
    let b: serde_json::Value = serde_json::from_str(&seen[3].body).unwrap();
    assert_eq!(b["stream"], true);
    assert!(b["messages"].to_string().contains("PLANSTREAM"));
    assert!(err.contains("10 in / 5 out"), "{err}");
}

#[test]
fn without_yes_a_non_tty_fails_before_any_chat_call() {
    let (base, rx) = serve(vec![(200, PRICE.into()), (200, PRICE.into())]);
    let (code, out, err) = run(&base, &["task", "x"], None);
    assert_ne!(code, 0, "{out}{err}");
    assert!(err.contains("--yes"), "{err}");
    for _ in 0..2 {
        let seen = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(seen.path.contains("/v1/models/"), "{}", seen.path);
    }
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn a_failed_plan_stops_before_the_do_call() {
    let (base, rx) = serve(vec![
        (200, PRICE.into()),
        (200, PRICE.into()),
        (500, r#"{"error":{"message":"plan down"}}"#.into()),
    ]);
    let (code, _, err) = run(&base, &["task", "x", "--yes", "--json"], None);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("plan down"), "{err}");
    for _ in 0..3 {
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn empty_request_is_a_usage_error() {
    let (base, _rx) = serve(vec![]);
    let (code, _, err) = run(&base, &["task"], None);
    assert_eq!(code, 2, "{err}");
}
