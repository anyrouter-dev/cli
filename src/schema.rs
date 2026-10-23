//! A small JSON Schema checker for `anyr publish`.
//!
//! It covers what a typical zod-generated schema uses: `type`, `enum`,
//! `const`, `properties`, `required`, `additionalProperties`, `items`,
//! `prefixItems`, size bounds, `anyOf` / `oneOf` / `allOf` / `not` and local
//! `$ref`. `pattern` and `format` are ignored here; the server stays the
//! authority and re-checks every submission. Output is one human line per
//! problem, each naming the JSON path.

use serde_json::Value;

/// Every way `value` breaks `schema`, as `path: message` lines. Empty = valid.
pub fn validate(schema: &Value, value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    check(schema, schema, value, "$", 0, &mut errors);
    errors
}

const MAX_DEPTH: usize = 64;

fn check(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
    depth: usize,
    out: &mut Vec<String>,
) {
    if depth > MAX_DEPTH {
        out.push(format!("{path}: nested too deeply"));
        return;
    }
    let Some(obj) = schema.as_object() else {
        if schema == &Value::Bool(false) {
            out.push(format!("{path}: not allowed"));
        }
        return;
    };
    if let Some(target) = obj
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix('#'))
    {
        match root.pointer(target) {
            Some(resolved) => check(root, resolved, value, path, depth + 1, out),
            None => out.push(format!("{path}: schema reference #{target} not found")),
        }
        return;
    }

    if let Some(t) = obj.get("type") {
        let names: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !names.is_empty() && !names.iter().any(|n| type_matches(n, value)) {
            out.push(format!(
                "{path}: expected {}, got {}",
                names.join(" or "),
                type_name(value)
            ));
            return;
        }
    }
    if let Some(Value::Array(options)) = obj.get("enum") {
        if !options.contains(value) {
            out.push(format!("{path}: must be one of {}", list(options)));
        }
    }
    if let Some(c) = obj.get("const") {
        if c != value {
            out.push(format!("{path}: must equal {c}"));
        }
    }

    match value {
        Value::Object(map) => {
            if let Some(Value::Array(req)) = obj.get("required") {
                for k in req.iter().filter_map(Value::as_str) {
                    if !map.contains_key(k) {
                        out.push(format!("{path}.{k}: required"));
                    }
                }
            }
            let props = obj.get("properties").and_then(Value::as_object);
            for (k, v) in map {
                let child = format!("{path}.{k}");
                if let Some(sub) = props.and_then(|p| p.get(k)) {
                    check(root, sub, v, &child, depth + 1, out);
                } else {
                    match obj.get("additionalProperties") {
                        Some(Value::Bool(false)) => out.push(format!("{child}: unknown field")),
                        Some(sub @ Value::Object(_)) => check(root, sub, v, &child, depth + 1, out),
                        _ => {}
                    }
                }
            }
        }
        Value::Array(items) => {
            bound(
                obj,
                "minItems",
                items.len() as f64,
                path,
                "items",
                true,
                out,
            );
            bound(
                obj,
                "maxItems",
                items.len() as f64,
                path,
                "items",
                false,
                out,
            );
            let prefix = obj.get("prefixItems").and_then(Value::as_array);
            for (i, v) in items.iter().enumerate() {
                let child = format!("{path}[{i}]");
                if let Some(sub) = prefix.and_then(|p| p.get(i)) {
                    check(root, sub, v, &child, depth + 1, out);
                } else if let Some(sub) = obj.get("items") {
                    check(root, sub, v, &child, depth + 1, out);
                }
            }
        }
        Value::String(s) => {
            let n = s.chars().count() as f64;
            bound(obj, "minLength", n, path, "characters", true, out);
            bound(obj, "maxLength", n, path, "characters", false, out);
        }
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(f64::NAN);
            num(obj, "minimum", x, path, |x, l| x < l, "at least", out);
            num(obj, "maximum", x, path, |x, l| x > l, "at most", out);
            num(
                obj,
                "exclusiveMinimum",
                x,
                path,
                |x, l| x <= l,
                "greater than",
                out,
            );
            num(
                obj,
                "exclusiveMaximum",
                x,
                path,
                |x, l| x >= l,
                "less than",
                out,
            );
        }
        _ => {}
    }

    if let Some(Value::Array(all)) = obj.get("allOf") {
        for sub in all {
            check(root, sub, value, path, depth + 1, out);
        }
    }
    if let Some(Value::Array(any)) = obj.get("anyOf") {
        if !any.iter().any(|s| passes(root, s, value, path, depth)) {
            out.push(format!("{path}: matches none of the allowed shapes"));
        }
    }
    if let Some(Value::Array(one)) = obj.get("oneOf") {
        let hits = one
            .iter()
            .filter(|s| passes(root, s, value, path, depth))
            .count();
        if hits != 1 {
            out.push(format!(
                "{path}: must match exactly one allowed shape, matches {hits}"
            ));
        }
    }
    if let Some(sub) = obj.get("not") {
        if passes(root, sub, value, path, depth) {
            out.push(format!("{path}: matches a disallowed shape"));
        }
    }
}

fn passes(root: &Value, schema: &Value, value: &Value, path: &str, depth: usize) -> bool {
    let mut scratch = Vec::new();
    check(root, schema, value, path, depth + 1, &mut scratch);
    scratch.is_empty()
}

fn bound(
    obj: &serde_json::Map<String, Value>,
    key: &str,
    got: f64,
    path: &str,
    unit: &str,
    is_min: bool,
    out: &mut Vec<String>,
) {
    if let Some(limit) = obj.get(key).and_then(Value::as_f64) {
        if (is_min && got < limit) || (!is_min && got > limit) {
            let word = if is_min { "at least" } else { "at most" };
            out.push(format!("{path}: needs {word} {limit} {unit}, has {got}"));
        }
    }
}

fn num(
    obj: &serde_json::Map<String, Value>,
    key: &str,
    x: f64,
    path: &str,
    bad: fn(f64, f64) -> bool,
    word: &str,
    out: &mut Vec<String>,
) {
    if let Some(limit) = obj.get(key).and_then(Value::as_f64) {
        if bad(x, limit) {
            out.push(format!("{path}: must be {word} {limit}, got {x}"));
        }
    }
}

fn type_matches(name: &str, v: &Value) -> bool {
    match name {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        _ => true,
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn list(options: &[Value]) -> String {
    options
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "required": ["title", "parts"],
            "additionalProperties": false,
            "properties": {
                "title": {"type": "string", "minLength": 2, "maxLength": 10},
                "kind": {"enum": ["a", "b"]},
                "parts": {"type": "array", "minItems": 1, "items": {"$ref": "#/$defs/part"}}
            },
            "$defs": {"part": {"type": "object", "required": ["n"], "properties": {"n": {"type": "integer", "minimum": 1}}}}
        })
    }

    #[test]
    fn accepts_a_valid_document() {
        let v = json!({"title": "Lock", "kind": "a", "parts": [{"n": 2}]});
        assert!(validate(&schema(), &v).is_empty());
    }

    #[test]
    fn reports_every_problem_with_its_path() {
        let v = json!({"title": "x", "kind": "z", "extra": 1, "parts": [{"n": 0}, {}]});
        let errs = validate(&schema(), &v);
        let all = errs.join("\n");
        assert!(
            all.contains("$.title: needs at least 2 characters"),
            "{all}"
        );
        assert!(all.contains("$.kind: must be one of"), "{all}");
        assert!(all.contains("$.extra: unknown field"), "{all}");
        assert!(all.contains("$.parts[0].n: must be at least 1"), "{all}");
        assert!(all.contains("$.parts[1].n: required"), "{all}");
    }

    #[test]
    fn type_mismatch_stops_descending() {
        let errs = validate(&schema(), &json!("nope"));
        assert_eq!(errs, vec!["$: expected object, got string".to_string()]);
    }

    #[test]
    fn any_of_and_nullable_types() {
        let s = json!({"anyOf": [{"type": "string"}, {"type": "null"}]});
        assert!(validate(&s, &json!(null)).is_empty());
        assert_eq!(validate(&s, &json!(3)).len(), 1);
        let t = json!({"type": ["integer", "null"]});
        assert!(validate(&t, &json!(4.0)).is_empty());
        assert!(!validate(&t, &json!(4.5)).is_empty());
    }

    #[test]
    fn dangling_ref_is_reported_not_ignored() {
        let errs = validate(&json!({"$ref": "#/$defs/missing"}), &json!(1));
        assert!(errs[0].contains("not found"), "{errs:?}");
    }
}
