//! `anyr embed` against a local scripted server.

mod common;
use common::{run, serve};
use std::time::Duration;

const EMB: &str = r#"{"model":"e/ok","data":[{"index":0,"embedding":[0.1,0.2,0.3]},{"index":1,"embedding":[0.4,0.5,0.6]}],"usage":{"total_tokens":4}}"#;
const CATALOG: &str = r#"{"data":[{"id":"t/chat","category":"text"},{"id":"e/ok","category":"embedding","pricing":{}}]}"#;

#[test]
fn explicit_model_posts_one_batched_request() {
    let (base, rx) = serve(vec![(200, EMB.into())]);
    let (code, out, err) = run(
        &base,
        &["embed", "--model", "e/ok", "--dimensions", "256", "a", "b"],
        None,
    );
    assert_eq!(code, 0, "{out}{err}");
    let seen = rx.recv().unwrap();
    assert_eq!(
        (seen.method.as_str(), seen.path.as_str()),
        ("POST", "/api/v1/embeddings")
    );
    let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
    assert_eq!(body["input"], serde_json::json!(["a", "b"]));
    assert_eq!(body["model"], "e/ok");
    assert_eq!(body["dimensions"], 256);
    assert!(
        out.contains("dimensions") && out.contains("0.1000"),
        "{out}"
    );
}

#[test]
fn default_model_comes_from_the_catalog_and_stdin_is_read() {
    let (base, rx) = serve(vec![(200, CATALOG.into()), (200, EMB.into())]);
    let (code, out, err) = run(&base, &["embed", "--json", "-"], Some("hi\nthere\n"));
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(rx.recv().unwrap().path, "/api/v1/models");
    let body: serde_json::Value = serde_json::from_str(&rx.recv().unwrap().body).unwrap();
    assert_eq!(body["model"], "e/ok");
    assert_eq!(body["input"], serde_json::json!(["hi", "there"]));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["data"][0]["embedding"].as_array().unwrap().len(), 3);
}

#[test]
fn file_inputs_are_one_per_line() {
    let dir = std::env::temp_dir().join("anyr-embed-file-test");
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("notes.txt");
    std::fs::write(&f, "one\n\ntwo\n").unwrap();
    let (base, rx) = serve(vec![(200, EMB.into())]);
    let (code, _, err) = run(
        &base,
        &["embed", "--model", "e/ok", "--file", f.to_str().unwrap()],
        None,
    );
    assert_eq!(code, 0, "{err}");
    let body: serde_json::Value = serde_json::from_str(&rx.recv().unwrap().body).unwrap();
    assert_eq!(body["input"], serde_json::json!(["one", "two"]));
}

#[test]
fn non_embedding_model_error_is_passed_through() {
    let (base, _rx) = serve(vec![(
        400,
        r#"{"error":{"message":"model t/chat is not an embedding model"}}"#.into(),
    )]);
    let (code, _, err) = run(&base, &["embed", "--model", "t/chat", "x"], None);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.starts_with("error:") && err.contains("not an embedding model"),
        "{err}"
    );
}

#[test]
fn no_input_is_exit_2_and_sends_nothing() {
    let (base, rx) = serve(vec![(200, EMB.into())]);
    let (code, _, err) = run(&base, &["embed"], None);
    assert_eq!(code, 2, "{err}");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn restricted_key_exits_4() {
    let (base, _rx) = serve(vec![(403, r#"{"error":{"message":"no scope"}}"#.into())]);
    let (code, _, err) = run(&base, &["embed", "--model", "e/ok", "x"], None);
    assert_eq!(code, 4, "{err}");
}
