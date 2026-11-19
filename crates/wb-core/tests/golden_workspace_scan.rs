mod common;

use common::*;
use serde_json::Value;
use wb_core::error::CoreError;
use wb_core::{Workflow, Workspace};

fn ws() -> Workspace {
    Workspace::open(&fixtures().join("work"), Workflow::preset("classic-es").unwrap(), None).unwrap()
}

fn v1<T: serde::Serialize>(r: Result<T, CoreError>) -> Result<Value, CoreError> {
    r.map(|v| to_v1(serde_json::to_value(v).unwrap()))
}

#[test]
fn list_items() {
    let g = golden("workspace_scan");
    let mut items = ws().list_items().unwrap();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    assert_json(&to_v1(serde_json::to_value(&items).unwrap()), &g["list_items"], "list_items");
}

#[test]
fn list_deleted() {
    let g = golden("workspace_scan");
    let mut deleted = ws().list_deleted().unwrap();
    deleted.sort_by(|a, b| a.path.cmp(&b.path));
    assert_json(&to_v1(serde_json::to_value(&deleted).unwrap()), &g["list_deleted"], "list_deleted");
}

#[test]
fn paths_and_reads() {
    let g = golden("workspace_scan");
    let ws = ws();
    for case in arr(&g["folder_of"]) {
        let name = s(&case["name"]);
        assert_outcome(&ws.folder_of(name).map(|p| ws.rel(&p)), case, &format!("folder_of {name}"));
    }
    for case in arr(&g["safe_path"]) {
        let mut rel = s(&case["rel"]).to_string();
        if rel == "C:/Windows" && !cfg!(windows) {
            rel = "/etc".into(); // el generador usa la raíz absoluta de cada SO
        }
        assert_outcome(&ws.safe_path(&rel).map(|p| ws.rel(&p)), case, &format!("safe_path {rel}"));
    }
    for case in arr(&g["read_file"]) {
        let rel = s(&case["rel"]);
        assert_outcome(&ws.read_file(rel), case, &format!("read_file {rel}"));
    }
    for case in arr(&g["read_media"]) {
        let rel = s(&case["rel"]);
        assert_outcome(&v1(ws.read_media(rel)), case, &format!("read_media {rel}"));
    }
    for case in arr(&g["get_meta"]) {
        let name = s(&case["name"]);
        assert_outcome(&ws.get_meta(name), case, &format!("get_meta {name}"));
    }
}
