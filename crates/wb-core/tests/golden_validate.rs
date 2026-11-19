mod common;

use common::*;
use indexmap::IndexMap;
use wb_core::Workflow;
use wb_core::text::{self, PlanGap};
use wb_core::validate::{parse_hours, validate_fields};

fn classic() -> Workflow {
    Workflow::preset("classic-es").unwrap()
}

#[test]
fn fields() {
    let wf = classic();
    for case in arr(&golden("validate")) {
        let fields: IndexMap<String, serde_json::Value> =
            case["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        assert_outcome(&validate_fields(&wf, &fields), case, &case["fields"].to_string());
    }
}

#[test]
fn hours() {
    for case in arr(&golden("misc")["horas"]) {
        assert_eq!(parse_hours(&py_str(&case["value"])), case["result"].as_f64(), "{}", case["value"]);
    }
}

#[test]
fn slugify() {
    for case in arr(&golden("misc")["slugify"]) {
        let max = case.get("max_len").and_then(|m| m.as_u64()).unwrap_or(40) as usize;
        assert_eq!(text::slugify(s(&case["text"]), max, "wi"), s(&case["slug"]), "{}", case["text"]);
    }
}

#[test]
fn sections() {
    for case in arr(&golden("misc")["extract_section"]) {
        let keywords: Vec<&str> = arr(&case["keywords"]).iter().map(s).collect();
        assert_eq!(
            text::extract_section(s(&case["body"]), &keywords),
            s(&case["section"]),
            "{}",
            case["body"]
        );
    }
}

#[test]
fn plan_gaps() {
    let wf = classic();
    for case in arr(&golden("misc")["plan_gaps"]) {
        let gaps: Vec<&str> = text::plan_gaps(&wf, s(&case["body"]))
            .into_iter()
            .map(|g| match g {
                PlanGap::Plan => "plan",
                PlanGap::Tasks => "pendientes",
            })
            .collect();
        let expected: Vec<&str> = arr(&case["gaps"]).iter().map(s).collect();
        assert_eq!(gaps, expected, "{}", case["body"]);
    }
}
