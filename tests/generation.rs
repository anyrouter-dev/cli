//! `anyr generation` against a local scripted server.

mod common;
use common::{run, serve, KEY};
use std::time::Duration;

const BODY: &str = r#"{"id":"gen_1","model":"z-ai/glm-4.7","provider_name":"nvidia","status":"success","finish_reason":"stop","tokens_prompt":12,"tokens_completion":34,"total_cost":0.0012,"latency_ms":840,"cache_hit":false,"created_at":"2026-10-24T01:51:16Z"}"#;

#[test]
fn prints_details_and_the_alias_hits_the_same_route() {
    let (base, rx) = serve(vec![(200, BODY.into()), (200, BODY.into())]);
    let (code, out, err) = run(&base, &["generation", "gen_1"], None);
    assert_eq!(code, 0, "{out}{err}");
    let seen = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.path, "/api/v1/generation/gen_1");
    assert_eq!(seen.auth, format!("Bearer {KEY}"));
    for want in [
        "z-ai/glm-4.7",
        "nvidia",
        "$0.0012",
        "840ms",
        "success (stop)",
    ] {
        assert!(out.contains(want), "missing {want}: {out}");
    }
    let (code, _, _) = run(&base, &["gen", "gen_1"], None);
    assert_eq!(code, 0);
    assert_eq!(rx.recv().unwrap().path, "/api/v1/generation/gen_1");
}

#[test]
fn json_prints_the_raw_body() {
    let (base, _rx) = serve(vec![(200, BODY.into())]);
    let (code, out, _) = run(&base, &["generation", "gen_1", "--json"], None);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["total_cost"], 0.0012);
}

#[test]
fn unknown_id_exits_1_with_the_server_message_on_stderr() {
    let (base, _rx) = serve(vec![(
        404,
        r#"{"error":{"message":"Generation not found: nope","code":"generation_not_found"}}"#
            .into(),
    )]);
    let (code, out, err) = run(&base, &["generation", "nope"], None);
    assert_eq!(code, 1, "{err}");
    assert!(out.is_empty(), "{out}");
    assert!(err.starts_with("error:"), "{err}");
    assert!(err.contains("Generation not found"), "{err}");
}

#[test]
fn missing_id_is_a_usage_error_and_sends_nothing() {
    let (base, rx) = serve(vec![(200, BODY.into())]);
    let (code, _, err) = run(&base, &["generation"], None);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("Usage"), "{err}");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}
