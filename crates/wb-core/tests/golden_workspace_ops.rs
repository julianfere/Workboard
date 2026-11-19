//! La secuencia de escrituras de `workspace_ops.json`, en orden, sobre una copia de `fixtures/work`.

mod common;

use common::*;
use indexmap::IndexMap;
use serde_json::{Value as Json, json};
use wb_core::error::CoreError;
use wb_core::frontmatter::{self, Value};
use wb_core::{Workflow, Workspace};

fn fields(v: &Json) -> IndexMap<String, Value> {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            let value = match v {
                Json::Array(items) => Value::List(items.iter().map(py_str).collect()),
                other => Value::Text(py_str(other)),
            };
            (k.clone(), value)
        })
        .collect()
}

fn v1<T: serde::Serialize>(r: Result<T, CoreError>) -> Result<Json, CoreError> {
    r.map(|v| to_v1(serde_json::to_value(v).unwrap()))
}

fn run(ws: &Workspace, op: &str, a: &[Json]) -> Result<Json, CoreError> {
    let st = |i: usize| s(&a[i]);
    let idx = |i: usize| a[i].as_i64().unwrap();
    let opt = |i: usize| a.get(i).and_then(|v| v.as_str());
    match op {
        "update_meta" => v1(ws.update_meta(st(0), &fields(&a[1]))),
        "add_task" => v1(ws.add_task(st(0), st(1))),
        "set_task" => v1(ws.set_task(st(0), idx(1), a[2].as_bool().unwrap(), opt(3))),
        "remove_task" => v1(ws.remove_task(st(0), idx(1), opt(2))),
        "edit_task" => v1(ws.edit_task(st(0), idx(1), st(2), opt(3))),
        "move_folder" => v1(ws.move_folder(st(0), st(1))),
        "trash" => v1(ws.trash(st(0))),
        "untrash" => v1(ws.untrash(st(0), st(1))),
        "rename_folder" => v1(ws.rename_folder(st(0), st(1))),
        "create_folder" => v1(ws.create_folder(st(0)).map(|p| ws.rel(&p))),
        other => panic!("unknown op {other}"),
    }
}

#[test]
fn write_sequence() {
    let g = golden("workspace_ops");
    let tmp = TempDir::new("ops");
    let root = tmp.copy_of(&fixtures().join("work"));
    let ws = Workspace::open(&root, Workflow::preset("classic-es").unwrap(), Some(tmp.0.join("cfg/backups")))
        .unwrap();
    let mut readmes = std::collections::BTreeMap::new();
    let snapshot = |root: &std::path::Path| -> std::collections::BTreeMap<String, String> {
        tree(root)
            .into_iter()
            .filter(|p| p.ends_with("README.md"))
            .map(|p| {
                let text = String::from_utf8_lossy(&std::fs::read(root.join(&p)).unwrap()).into_owned();
                (p, text)
            })
            .collect()
    };
    readmes.extend(snapshot(&root));
    for step in arr(&g["steps"]) {
        let desc = s(&step["desc"]);
        let op = s(&step["op"]);
        let args = arr(&step["args"]).clone();
        if step.get("ok_pattern").is_some() {
            // Como en el generador: se vuelve a crear la carpeta para eliminar un nombre que ya está en _deleted.
            ws.create_folder(s(&args[0])).unwrap();
        }
        let result = run(&ws, op, &args);
        if let Some(pattern) = step.get("ok_pattern") {
            let re = regex::Regex::new(s(pattern)).unwrap();
            let ok = result.as_ref().ok().and_then(|v| v.as_str()).is_some_and(|v| re.is_match(v));
            assert_eq!(ok, step["matches"].as_bool().unwrap(), "{desc}: {result:?}");
            continue;
        }
        assert_outcome(&result, step, desc);
        assert_eq!(
            tree(&root),
            arr(&step["tree"]).iter().map(|t| s(t).to_string()).collect::<Vec<_>>(),
            "{desc}: tree"
        );
        let now = snapshot(&root);
        let changed: serde_json::Map<String, Json> = now
            .iter()
            .filter(|(p, t)| readmes.get(*p) != Some(*t))
            .map(|(p, t)| (p.clone(), json!(t)))
            .collect();
        assert_json(&Json::Object(changed), &step["readmes_changed"], &format!("{desc}: READMEs"));
        readmes = now;
    }

    // Versiones: cada escritura dejó una copia; restaurar la más vieja vuelve al fixture.
    let name = "wi111111-login-con-sso";
    let versions = ws.list_versions(name).unwrap();
    let expected = &g["versions"];
    assert_eq!(versions.len() as u64, expected["count_before_restore"].as_u64().unwrap());
    let oldest = &versions.last().unwrap().id;
    let original = ws.read_version(name, oldest).unwrap();
    let fixture = frontmatter::read(&fixtures().join("work").join(name).join("README.md")).unwrap();
    assert_eq!(original == fixture, expected["oldest_equals_fixture"].as_bool().unwrap());
    ws.restore_version(name, oldest).unwrap();
    let restored = String::from_utf8(std::fs::read(root.join(name).join("README.md")).unwrap()).unwrap();
    assert_eq!(restored == original, expected["restored_equals_oldest"].as_bool().unwrap());
    assert_eq!(
        ws.list_versions(name).unwrap().len() as u64,
        expected["count_after_restore"].as_u64().unwrap()
    );
    assert_outcome(&ws.read_version(name, "../x"), &expected["invalid_id"], "invalid version id");
    assert_outcome(
        &ws.read_version(name, "20000101-000000-000000000"),
        &expected["missing_id"],
        "missing version",
    );

    // state_log.jsonl: un renglón por cambio de estado, en el formato de la v1.
    let log = std::fs::read_to_string(tmp.0.join("cfg/state_log.jsonl")).unwrap();
    let rows: Vec<Json> = log
        .lines()
        .map(|l| {
            let mut v: Json = serde_json::from_str(l).unwrap();
            v.as_object_mut().unwrap().retain(|k, _| k != "ts" && k != "root");
            v
        })
        .collect();
    assert_json(&Json::Array(rows), &g["state_log"], "state_log");
}

#[test]
fn backup_rotation() {
    let g = &golden("workspace_ops")["backup_rotation"];
    let tmp = TempDir::new("rotation");
    let root = tmp.copy_of(&fixtures().join("work"));
    let ws = Workspace::open(&root, Workflow::preset("classic-es").unwrap(), Some(tmp.0.join("cfg/backups")))
        .unwrap();
    let writes = g["writes"].as_u64().unwrap();
    for i in 0..writes {
        ws.update_meta("grp-checkout", &fields(&json!({"notas": format!("v{i}")}))).unwrap();
    }
    let versions = ws.list_versions("grp-checkout").unwrap();
    assert_eq!(versions.len() as u64, g["kept"].as_u64().unwrap());
    let oldest = ws.read_version("grp-checkout", &versions.last().unwrap().id).unwrap();
    assert_eq!(frontmatter::split(&oldest).0["notas"], Value::Text(s(&g["oldest_notas"]).into()));
}
