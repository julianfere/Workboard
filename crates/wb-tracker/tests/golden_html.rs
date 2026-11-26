//! `html_to_text` contra los casos de la v1 (`tests/golden/cases/misc.json`).

use serde_json::Value;
use wb_tracker::html::html_to_text;

#[test]
fn html_to_text_matches_v1() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/golden/cases/misc.json");
    let cases: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let list = cases["html_to_text"].as_array().unwrap();
    assert!(!list.is_empty());
    for c in list {
        assert_eq!(html_to_text(c["html"].as_str().unwrap()), c["text"].as_str().unwrap(), "{}", c["html"]);
    }
}
