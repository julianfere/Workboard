mod common;

use common::*;
use serde_json::{Value, json};
use wb_core::frontmatter::{self, Meta};

fn meta_json(meta: &Meta) -> Value {
    serde_json::to_value(meta).unwrap()
}

fn meta_from(v: &Value) -> Meta {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            let value = match v {
                Value::Array(items) => frontmatter::Value::List(items.iter().map(py_str).collect()),
                other => frontmatter::Value::Text(py_str(other)),
            };
            (k.clone(), value)
        })
        .collect()
}

#[test]
fn split() {
    for case in arr(&golden("frontmatter")["split"]) {
        let (meta, body) = frontmatter::split(s(&case["text"]));
        assert_json(&json!([meta_json(&meta), body]), &case["ok"], s(&case["text"]));
        // El orden de las claves también es parte del formato.
        let keys: Vec<_> = meta.keys().cloned().collect();
        let expected: Vec<_> = case["ok"][0].as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, expected);
    }
}

#[test]
fn join() {
    for case in arr(&golden("frontmatter")["join"]) {
        let out = frontmatter::join(&meta_from(&case["meta"]), s(&case["body"]));
        assert_eq!(out, s(&case["ok"]), "{}", case["meta"]);
    }
}

#[test]
fn roundtrip() {
    for case in arr(&golden("frontmatter")["roundtrip"]) {
        let (meta, body) = frontmatter::split(s(&case["text"]));
        assert_eq!(frontmatter::join(&meta, &body), s(&case["result"]), "{}", case["text"]);
    }
}

#[test]
fn decode() {
    for case in arr(&golden("frontmatter")["read_text"]) {
        let hex = s(&case["bytes_hex"]);
        let bytes: Vec<u8> =
            (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        assert_eq!(frontmatter::decode(&bytes), s(&case["text"]), "{}", case["name"]);
    }
}
