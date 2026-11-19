//! Lectura de los golden files de la v1 (`tests/golden/cases/*.json`).
#![allow(dead_code)]

use std::path::PathBuf;

use serde_json::Value;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

pub fn golden(name: &str) -> Value {
    let path = repo_root().join("tests/golden/cases").join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("golden file")).expect("golden JSON")
}

pub fn fixtures() -> PathBuf {
    repo_root().join("tests/golden/fixtures")
}

pub fn arr(v: &Value) -> &Vec<Value> {
    v.as_array().unwrap_or_else(|| panic!("expected array, got {v}"))
}

pub fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("expected string, got {v}"))
}

/// `str(x)` de Python para los valores que aparecen en los casos (None → "").
pub fn py_str(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => (if *b { "True" } else { "False" }).into(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else {
                let f = n.as_f64().unwrap();
                if f.fract() == 0.0 { format!("{f:.1}") } else { f.to_string() }
            }
        }
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Nombre de la excepción de Python que lanzaba la v1 para cada clase de error.
pub fn py_exception(kind: wb_core::error::ErrorKind) -> &'static str {
    use wb_core::error::ErrorKind::*;
    match kind {
        Invalid => "ValueError",
        NotFound => "FileNotFoundError",
        Exists => "FileExistsError",
        Permission => "PermissionError",
        Conflict => "RuntimeError",
        Io => "OSError",
    }
}

/// Mensaje en español (el de la v1) de un error del core.
pub fn es_message(err: &wb_core::error::CoreError) -> String {
    let params: Vec<(&str, &str)> = err.params.iter().map(|(k, v)| (*k, v.as_str())).collect();
    wb_i18n::t("es", err.code, &params)
}

/// Verifica que `result` sea el error del caso (`{"error": tipo, "message": ...}`) o su valor `ok`.
#[track_caller]
pub fn assert_outcome<T: serde::Serialize>(
    result: &Result<T, wb_core::error::CoreError>,
    case: &Value,
    ctx: &str,
) {
    match (result, case.get("ok"), case.get("error")) {
        (Ok(v), Some(ok), _) => assert_json(&serde_json::to_value(v).unwrap(), ok, ctx),
        (Err(e), _, Some(err)) => {
            assert_eq!(py_exception(e.kind), s(err), "{ctx}: error type ({e})");
            assert_eq!(es_message(e), s(&case["message"]), "{ctx}: message");
        }
        (Ok(v), None, _) => {
            panic!("{ctx}: expected error {}, got {}", case["error"], serde_json::to_value(v).unwrap())
        }
        (Err(e), _, None) => panic!("{ctx}: unexpected error {e}"),
    }
}

/// Compara como JSON y, si difiere, muestra los dos lados enteros.
#[track_caller]
pub fn assert_json(ours: &Value, expected: &Value, ctx: &str) {
    if ours != expected {
        panic!(
            "{ctx}\n--- ours ---\n{}\n--- expected ---\n{}",
            serde_json::to_string_pretty(ours).unwrap(),
            serde_json::to_string_pretty(expected).unwrap()
        );
    }
}
