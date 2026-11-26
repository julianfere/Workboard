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

/// Carpeta temporal propia de un test (se borra al soltarla).
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "wb-test-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dunce::canonicalize(&dir).unwrap())
    }

    /// Copia de un fixture adentro de esta carpeta.
    pub fn copy_of(&self, src: &std::path::Path) -> PathBuf {
        let dest = self.0.join(src.file_name().unwrap());
        copy_dir(src, &dest);
        dest
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(src: &std::path::Path, dest: &std::path::Path) {
    std::fs::create_dir_all(dest).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &dest.join(e.file_name()));
        } else {
            std::fs::copy(&p, dest.join(e.file_name())).unwrap();
        }
    }
}

/// Archivos del árbol como `os.walk` ordenado de la v1: los archivos de cada carpeta y después sus
/// subcarpetas, todo en orden.
pub fn tree(root: &std::path::Path) -> Vec<String> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        let mut files = vec![];
        let mut dirs = vec![];
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() { dirs.push(name) } else { files.push(name) }
        }
        files.sort();
        dirs.sort();
        let prefix = dir.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        for f in files {
            out.push(if prefix.is_empty() { f } else { format!("{prefix}/{f}") });
        }
        for d in dirs {
            walk(root, &dir.join(d), out);
        }
    }
    let mut out = vec![];
    walk(root, root, &mut out);
    out
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

/// Pasa la salida del port (nombres genéricos) a la forma de la v1 que tienen los golden: `ids`→`wis`,
/// `type`→`tipo`, `status`→`estado`, kind `item`→`wi`, gap `tasks`→`pendientes`, y saca los campos de
/// tiempo (`created`, `last`, `mtime`, `deleted`, `ts`).
pub fn to_v1(v: Value) -> Value {
    match v {
        Value::Array(items) => Value::Array(items.into_iter().map(to_v1).collect()),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            let is_item = map.contains_key("hasReadme");
            for (k, v) in map {
                if ["created", "last", "mtime", "deleted", "ts", "original"].contains(&k.as_str()) {
                    continue;
                }
                let (k, v) = match k.as_str() {
                    "ids" => ("wis".to_string(), v),
                    "type" if is_item => ("tipo".to_string(), v),
                    "status" => ("estado".to_string(), v),
                    "kind" if is_item && v == "item" => (k, Value::String("wi".into())),
                    "planGaps" => (
                        k,
                        Value::Array(
                            v.as_array()
                                .unwrap()
                                .iter()
                                .map(|g| {
                                    if g == "tasks" { Value::String("pendientes".into()) } else { g.clone() }
                                })
                                .collect(),
                        ),
                    ),
                    // El frontmatter queda tal cual: sus claves son las del archivo.
                    "meta" => (k, v),
                    _ => (k, to_v1(v)),
                };
                out.insert(k, v);
            }
            Value::Object(out)
        }
        other => other,
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
