//! `anyr eval` against a local scripted server.

mod common;
use common::{run, serve};
use std::time::Duration;

fn ok(text: &str) -> String {
    format!(
        r#"{{"choices":[{{"message":{{"content":"{text}"}}}}],"usage":{{"prompt_tokens":3,"completion_tokens":4}}}}"#
    )
}

#[test]
fn n_models_make_n_non_streaming_requests() {
    let (base, rx) = serve(vec![(200, ok("A1")), (200, ok("A2")), (200, ok("A3"))]);
    let (code, out, err) = run(
        &base,
        &[
            "eval",
            "hello",
            "--model",
            "m/a,m/b,m/c",
            "--max-tokens",
            "50",
        ],
        None,
    );
    assert_eq!(code, 0, "{out}{err}");
    let mut models = Vec::new();
    for _ in 0..3 {
        let seen = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(seen.path, "/api/v1/chat/completions");
        let b: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
        assert_eq!(b["stream"], false);
        assert_eq!(b["max_tokens"], 50);
        assert_eq!(b["messages"][0]["content"], "hello");
        models.push(b["model"].as_str().unwrap().to_string());
    }
    models.sort();
    assert_eq!(models, ["m/a", "m/b", "m/c"]);
    for m in ["m/a", "m/b", "m/c"] {
        assert!(out.contains(m), "{out}");
    }
}

#[test]
fn one_failing_model_is_reported_and_the_others_still_print() {
    // concurrency 1 makes the scripted replies line up with model order.
    let (base, _rx) = serve(vec![
        (200, ok("first answer")),
        (500, r#"{"error":{"message":"upstream down"}}"#.into()),
        (200, ok("third answer")),
    ]);
    let (code, out, err) = run(
        &base,
        &["eval", "hi", "--model", "m/a,m/b,m/c", "--concurrency", "1"],
        None,
    );
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        out.contains("first answer") && out.contains("third answer"),
        "{out}"
    );
    assert!(out.contains("upstream down"), "{out}");
    let row = out.lines().find(|l| l.starts_with("m/b\t")).expect(&out);
    assert!(row.ends_with("error"), "{row}");
    assert!(
        out.lines()
            .any(|l| l.starts_with("m/c\t") && l.ends_with("ok")),
        "{out}"
    );
    assert!(err.contains("1 of 3 requests failed"), "{err}");
}

#[test]
fn json_prints_one_line_per_prompt_and_model_in_order() {
    let dir = std::env::temp_dir().join(format!("anyr-eval-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("cases.jsonl");
    std::fs::write(&file, "{\"prompt\":\"p1\"}\n{\"prompt\":\"p2\"}\n").unwrap();
    let (base, _rx) = serve(vec![
        (200, ok("a")),
        (200, ok("b")),
        (200, ok("c")),
        (200, ok("d")),
    ]);
    let (code, out, err) = run(
        &base,
        &[
            "eval",
            "--prompts",
            file.to_str().unwrap(),
            "--model",
            "m/a,m/b",
            "--concurrency",
            "1",
            "--json",
        ],
        None,
    );
    assert_eq!(code, 0, "{out}{err}");
    let rows: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let got: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|r| {
            (
                r["prompt"].as_str().unwrap(),
                r["model"].as_str().unwrap(),
                r["answer"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("p1", "m/a", "a"),
            ("p1", "m/b", "b"),
            ("p2", "m/a", "c"),
            ("p2", "m/b", "d")
        ]
    );
    assert_eq!(rows[0]["status"], "ok");
    assert_eq!(rows[0]["tokens_in"], 3);
}

#[test]
fn bad_input_is_a_usage_error_and_sends_nothing() {
    let (base, rx) = serve(vec![(200, ok("x"))]);
    let (code, _, err) = run(&base, &["eval", "hi"], None);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("--model"), "{err}");
    let (code, _, _) = run(&base, &["eval", "--model", "a"], None);
    assert_eq!(code, 2);
    let (code, _, err) = run(
        &base,
        &["eval", "hi", "--model", "a", "--concurrency", "0"],
        None,
    );
    assert_eq!(code, 2, "{err}");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}
