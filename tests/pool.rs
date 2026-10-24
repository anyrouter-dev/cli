//! `anyr pool` against a local scripted server.

mod common;
use common::{run, serve};
use std::process::{Command, Stdio};
use std::time::Duration;

const STATS: &str = r#"{"pool_enabled":true,"eligible_provider_count":2,"active_donors":9,"requests_today":1234,"credits_rewarded_total":12.5}"#;
const PROVIDERS: &str = r#"{"pool_enabled":true,"providers":[{"provider":"openai","key_count":7,"donor_count":4,"donors_masked":["duy•••t"]},{"provider":"groq","key_count":2,"donor_count":2,"donors_masked":[]}]}"#;
const FREE: &str = r#"{"backends":["groq","nvidia"]}"#;
const RELAY: &str = r#"{"pool_enabled":true,"totals":{"online_clients":3,"models_served":1},"models":[{"model":"apple/foundation-model","device_count":3}]}"#;

#[test]
fn stats_hits_the_three_pool_routes_and_prints_a_summary() {
    let (base, rx) = serve(vec![
        (200, STATS.into()),
        (200, PROVIDERS.into()),
        (200, FREE.into()),
    ]);
    let (code, out, err) = run(&base, &["pool", "stats"], None);
    assert_eq!(code, 0, "{out}{err}");
    let paths: Vec<String> = (0..3)
        .map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap().path)
        .collect();
    assert_eq!(
        paths,
        [
            "/api/v1/pool/stats",
            "/api/v1/pool/providers",
            "/api/v1/pool/free-coverage"
        ]
    );
    for want in [
        "keys in pool      9 (openai, groq)",
        "requests today    1234",
        "groq, nvidia",
    ] {
        assert!(out.contains(want), "missing {want}: {out}");
    }
}

#[test]
fn providers_and_free_and_relay_use_their_own_routes() {
    for (sub, route, body, want) in [
        ("providers", "/api/v1/pool/providers", PROVIDERS, "duy•••t"),
        ("free", "/api/v1/pool/free-coverage", FREE, "nvidia"),
        (
            "relay",
            "/api/v1/relay/pool/availability",
            RELAY,
            "apple/foundation-model",
        ),
    ] {
        let (base, rx) = serve(vec![(200, body.into())]);
        let (code, out, err) = run(&base, &["pool", sub], None);
        assert_eq!(code, 0, "{sub}: {out}{err}");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().path, route);
        assert!(out.contains(want), "{sub}: {out}");
    }
}

#[test]
fn json_prints_the_raw_body() {
    let (base, _rx) = serve(vec![(200, PROVIDERS.into())]);
    let (code, out, _) = run(&base, &["pool", "providers", "--json"], None);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["providers"][0]["key_count"], 7);
}

#[test]
fn stats_json_wraps_the_three_bodies() {
    let (base, _rx) = serve(vec![
        (200, STATS.into()),
        (200, PROVIDERS.into()),
        (200, FREE.into()),
    ]);
    let (code, out, _) = run(&base, &["pool", "stats", "--json"], None);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["stats"]["requests_today"], 1234);
    assert_eq!(v["free_coverage"]["backends"][1], "nvidia");
}

#[test]
fn works_without_any_key_because_the_endpoints_are_public() {
    let (base, rx) = serve(vec![(200, FREE.into())]);
    let out = Command::new(env!("CARGO_BIN_EXE_anyr"))
        .env("ANYR_NO_UPDATE", "1")
        .env("NO_COLOR", "1")
        .env_remove("ANYROUTER_API_KEY")
        .env_remove("ANYROUTER_MANAGEMENT_KEY")
        .env(
            "ANYROUTER_HOME",
            std::env::temp_dir().join("anyr-pool-test-empty"),
        )
        .args(["pool", "free", "--base-url", &base])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let seen = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(seen.auth, "");
}

#[test]
fn server_error_exits_1_on_stderr() {
    let (base, _rx) = serve(vec![(500, r#"{"error":{"message":"boom"}}"#.into())]);
    let (code, out, err) = run(&base, &["pool", "free"], None);
    assert_eq!(code, 1, "{err}");
    assert!(out.is_empty(), "{out}");
    assert!(err.starts_with("error:") && err.contains("boom"), "{err}");
}

#[test]
fn pool_help_lists_subcommands_and_keeps_the_launch_help() {
    let (base, _rx) = serve(vec![]);
    let (code, out, _) = run(&base, &["pool", "--help"], None);
    assert_eq!(code, 0);
    for want in [
        "pool stats",
        "pool providers",
        "pool free",
        "pool relay",
        "launch Poolside",
    ] {
        assert!(out.contains(want), "missing {want}: {out}");
    }
}
