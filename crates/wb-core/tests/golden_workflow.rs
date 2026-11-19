//! El preset `classic-es` contra los golden de la v1 (`tests/golden/cases/misc.json`).

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;
use wb_core::Workflow;

fn golden(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/cases")
        .join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("golden file")).expect("golden JSON")
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

fn classic() -> Workflow {
    Workflow::preset("classic-es").unwrap()
}

#[test]
fn constants_match_v1() {
    let c = &golden("misc")["constants"];
    let wf = classic();
    assert_eq!(wf.state_ids().collect::<Vec<_>>(), strings(&c["STATES"]));
    assert_eq!(
        wf.closed_states().collect::<BTreeSet<_>>(),
        strings(&c["CLOSED_STATES"]).iter().map(String::as_str).collect()
    );
    assert_eq!(wf.environments.iter().map(|e| e.id.clone()).collect::<Vec<_>>(), strings(&c["ENVS"]));
    assert_eq!(wf.types.iter().map(|t| t.id.clone()).collect::<Vec<_>>(), strings(&c["TYPES"]));
    assert_eq!(wf.files.kinds, strings(&c["KINDS"]));
    assert_eq!(
        wf.categories().map(|c| c.id.clone()).collect::<BTreeSet<_>>(),
        strings(&c["CATEGORIES"]).into_iter().collect()
    );
    assert_eq!(
        wf.categories().filter(|c| c.runnable).map(|c| c.id.clone()).collect::<BTreeSet<_>>(),
        strings(&c["RUNNABLE_CATS"]).into_iter().collect()
    );
    assert_eq!(u64::from(wf.period.start_day), c["PERIOD_DAY"].as_u64().unwrap());
    assert_eq!(wf.tasks.max_len as u64, c["MAX_TASK_LEN"].as_u64().unwrap());
    assert_eq!(wf.folders.group_prefix, c["GROUP_PREFIX"].as_str().unwrap());
    assert_eq!(wf.folders.deleted_dir, c["DELETED_DIR"].as_str().unwrap());
    assert_eq!(wf.plan.placeholder, c["PLAN_PLACEHOLDER"].as_str().unwrap());
    assert_eq!(wf.plan.min_chars as u64, c["PLAN_MIN_CHARS"].as_u64().unwrap());
    assert_eq!(wf.fields.editable(), strings(&c["EDITABLE"]));
}

#[test]
fn item_ids_from_folder_names() {
    let wf = classic();
    for case in golden("misc")["wi_re"].as_array().unwrap() {
        let folder = case["folder"].as_str().unwrap();
        assert_eq!(wf.item_ids_in_folder(folder), strings(&case["wis"]), "{folder}");
    }
}

#[test]
fn periods() {
    let wf = classic();
    for case in golden("misc")["periods"].as_array().unwrap() {
        let date = case["date"].as_str().unwrap();
        let p: Vec<u32> = date.split('-').map(|x| x.parse().unwrap()).collect();
        assert_eq!(wf.period_key(p[0] as i32, p[1], p[2]), case["period"].as_str().unwrap(), "{date}");
    }
}

#[test]
fn file_kinds_and_categories() {
    let wf = classic();
    for case in golden("misc")["files"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        assert_eq!(wf.file_kind(name), case["kind"].as_str(), "kind of {name}");
        assert_eq!(wf.file_category(name).id, case["cat"].as_str().unwrap(), "category of {name}");
    }
}
