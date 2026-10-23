//! `anyr models` filters and `models show` against a local scripted server.

mod common;
use common::{run, serve};

const CATALOG: &str = r#"{"object":"list","data":[
 {"id":"nvidia/free-a","owned_by":"nvidia","name":"Free A","category":"text","capabilities":["chat"],"created":300,"context_length":128000,"pricing":{"input_per_1m":0,"output_per_1m":0,"billing":"free"}},
 {"id":"z-ai/glm-4.7","owned_by":"zhipu ai","name":"GLM","category":"text","capabilities":["chat"],"created":100,"context_length":204800,"pricing":{"input_per_1m":0.14,"output_per_1m":0.28}},
 {"id":"baai/bge","owned_by":"baai","name":"BGE","category":"embedding","capabilities":["embedding"],"created":50,"context_length":512,"pricing":{"input_per_1m":0.005,"output_per_1m":0}}
]}"#;

#[test]
fn free_filter_lists_only_zero_priced_models() {
    let (base, rx) = serve(vec![(200, CATALOG.into())]);
    let (code, out, err) = run(&base, &["models", "--free"], None);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(rx.recv().unwrap().path, "/api/v1/models");
    assert!(
        out.contains("nvidia/free-a") && !out.contains("z-ai/glm-4.7"),
        "{out}"
    );
}

#[test]
fn type_and_json_print_the_filtered_list() {
    let (base, _rx) = serve(vec![(200, CATALOG.into())]);
    let (code, out, _) = run(&base, &["models", "--type", "embedding", "--json"], None);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["data"].as_array().unwrap().len(), 1);
    assert_eq!(v["data"][0]["id"], "baai/bge");
}

#[test]
fn sort_price_puts_the_cheapest_first() {
    let (base, _rx) = serve(vec![(200, CATALOG.into())]);
    let (code, out, _) = run(&base, &["models", "--sort", "price"], None);
    assert_eq!(code, 0);
    let first = out.lines().next().unwrap();
    assert!(first.starts_with("nvidia/free-a"), "{out}");
}

#[test]
fn bad_type_or_sort_is_a_usage_error() {
    let (base, _rx) = serve(vec![]);
    let (code, _, err) = run(&base, &["models", "--type", "image"], None);
    assert_eq!(code, 2, "{err}");
    let (code, _, err) = run(&base, &["models", "--sort", "vibes"], None);
    assert_eq!(code, 2, "{err}");
}

#[test]
fn show_prints_detail_and_unknown_id_exits_1() {
    let detail = r#"{"id":"z-ai/glm-4.7","name":"GLM-4.7","owned_by":"zhipu ai","category":"text","capabilities":["chat"],"context_length":204800,"architecture":{"input_modalities":["text"],"output_modalities":["text"]},"pricing":{"input_per_1m":0.14,"output_per_1m":0.28},"per_upstream":[{"backend":"deepinfra","pricing":{"input_per_1m":0.1,"output_per_1m":0.2}}]}"#;
    let (base, rx) = serve(vec![
        (200, detail.into()),
        (200, detail.into()),
        (404, r#"{"error":{"message":"not found"}}"#.into()),
    ]);
    let (code, out, err) = run(&base, &["models", "show", "z-ai/glm-4.7"], None);
    assert_eq!(code, 0, "{err}");
    assert_eq!(rx.recv().unwrap().path, "/api/v1/models/z-ai/glm-4.7");
    assert!(
        out.contains("$0.14 in / $0.28 out") && out.contains("deepinfra"),
        "{out}"
    );
    let (code, out, _) = run(&base, &["models", "show", "z-ai/glm-4.7", "--json"], None);
    assert_eq!(code, 0);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out).unwrap()["id"],
        "z-ai/glm-4.7"
    );
    let (code, _, err) = run(&base, &["models", "show", "nope/nope"], None);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.starts_with("error:") && err.contains("unknown model"),
        "{err}"
    );
}
