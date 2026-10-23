//! `anyr logs` against a local scripted server.

mod common;
use common::{run, serve, KEY};
use std::time::Duration;

const BODY: &str = r#"{"data":[
 {"created_at":"2026-10-24T01:51:16Z","model":"z-ai/glm-4.7","provider_id":"nvidia","status":"error","error_code":"upstream_429","input_tokens":10,"output_tokens":0,"cost_usd":0,"latency_ms":120}
]}"#;

#[test]
fn query_string_carries_limit_model_and_status_ok_as_success() {
    let (base, rx) = serve(vec![(200, BODY.into())]);
    let (code, out, err) = run(
        &base,
        &[
            "logs",
            "--limit",
            "5",
            "--model",
            "z-ai/glm-4.7",
            "--status",
            "ok",
        ],
        None,
    );
    assert_eq!(code, 0, "{out}{err}");
    let seen = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(seen.method, "GET");
    assert_eq!(
        seen.path,
        "/api/v1/logs?limit=5&model=z-ai/glm-4.7&status=success"
    );
    assert_eq!(seen.auth, format!("Bearer {KEY}"));
    assert!(out.contains("error (upstream_429)"), "{out}");
}

#[test]
fn json_prints_the_raw_body() {
    let (base, _rx) = serve(vec![(200, BODY.into())]);
    let (code, out, _) = run(&base, &["logs", "--json", "--status", "error"], None);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["data"].as_array().unwrap().len(), 1);
}

#[test]
fn limit_out_of_range_is_a_usage_error_and_sends_nothing() {
    let (base, rx) = serve(vec![(200, BODY.into())]);
    let (code, _, err) = run(&base, &["logs", "--limit", "500"], None);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("--limit"), "{err}");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn id_and_sessions_hit_their_routes() {
    let (base, rx) = serve(vec![
        (200, r#"{"data":{"id":"req_1","model":"a/b"}}"#.into()),
        (200, r#"{"data":[]}"#.into()),
    ]);
    let (code, out, _) = run(&base, &["logs", "req_1", "--json"], None);
    assert_eq!(code, 0);
    assert!(out.contains("req_1"));
    assert_eq!(rx.recv().unwrap().path, "/api/v1/logs/req_1");
    let (code, _, _) = run(&base, &["logs", "sessions", "--limit", "3"], None);
    assert_eq!(code, 0);
    assert_eq!(rx.recv().unwrap().path, "/api/v1/logs/sessions?limit=3");
}

#[test]
fn scope_error_exits_4_with_error_prefix() {
    let (base, _rx) = serve(vec![(403, r#"{"error":{"message":"forbidden"}}"#.into())]);
    let (code, _, err) = run(&base, &["logs"], None);
    assert_eq!(code, 4, "{err}");
    assert!(err.starts_with("error:"), "{err}");
}
