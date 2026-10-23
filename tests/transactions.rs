//! `anyr transactions` against a local scripted server.

mod common;
use common::{run, serve};
use std::time::Duration;

const BODY: &str = r#"{"data":[{"created_at":"2026-10-24T01:51:16Z","type":"purchase","amount":10,"balance_after":12.5,"description":"Top up"}]}"#;

#[test]
fn default_limit_is_20_and_type_filter_reaches_the_query() {
    let (base, rx) = serve(vec![(200, BODY.into()), (200, BODY.into())]);
    let (code, out, err) = run(&base, &["transactions"], None);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(
        rx.recv().unwrap().path,
        "/api/v1/credits/transactions?limit=20"
    );
    assert!(out.contains("+$10.00") && out.contains("$12.50"), "{out}");
    let (code, _, _) = run(
        &base,
        &["transactions", "--type", "purchase", "--limit", "100"],
        None,
    );
    assert_eq!(code, 0);
    assert_eq!(
        rx.recv().unwrap().path,
        "/api/v1/credits/transactions?limit=100&type=purchase"
    );
}

#[test]
fn json_is_the_raw_body() {
    let (base, _rx) = serve(vec![(200, BODY.into())]);
    let (code, out, _) = run(&base, &["transactions", "--json"], None);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["data"][0]["type"], "purchase");
}

#[test]
fn bad_limit_is_exit_2_and_sends_nothing() {
    let (base, rx) = serve(vec![(200, BODY.into())]);
    let (code, _, err) = run(&base, &["transactions", "--limit", "0"], None);
    assert_eq!(code, 2, "{err}");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn restricted_key_exits_4() {
    let (base, _rx) = serve(vec![(403, r#"{"error":{"message":"no scope"}}"#.into())]);
    let (code, _, err) = run(&base, &["transactions"], None);
    assert_eq!(code, 4, "{err}");
    assert!(err.starts_with("error:"), "{err}");
}
