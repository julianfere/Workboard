//! Tools: scripts que escribieron las sesiones y el usuario promovió para reusar.
//!
//! Promover copia el script a `<config>/tools/<nombre>/` con `tool.json` (formato de la v1: descripción,
//! uso, origen, sha256). La copia queda congelada: si el archivo cambia, no se corre hasta promoverla de
//! nuevo. El MCP las lista y las corre (`workboard_list_tools`, `workboard_run_tool`) solo dentro de una
//! sesión. Nada se escribe en la carpeta de trabajos: todo vive en la carpeta de config.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use wb_core::error::{CoreError, ErrorKind, Result};

/// Extensión → intérprete (lo resuelve `wb_platform::script_command`).
pub const INTERPRETERS: [(&str, &str); 7] = [
    (".py", "python"),
    (".ps1", "powershell"),
    (".sh", "bash"),
    (".mjs", "node"),
    (".cjs", "node"),
    (".js", "node"),
    (".bat", "cmd"),
];
pub const RUN_TIMEOUT: Duration = Duration::from_secs(300);
pub const OUTPUT_MAX: usize = 20_000;
pub const MAX_SCRIPT_BYTES: u64 = 512 * 1024;
/// Carpeta (dentro de `tools/`) adonde van las tools quitadas.
pub const REMOVED_DIR: &str = "_removed";

static NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9_-]{1,40}$").expect("regex"));

fn err(kind: ErrorKind, code: &'static str, params: Vec<(&'static str, String)>) -> CoreError {
    CoreError::new(kind, code, params)
}

pub fn tools_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("tools")
}

/// Intérprete de un script por su extensión.
pub fn interpreter_of(path: &Path) -> Option<&'static str> {
    let ext = format!(".{}", path.extension()?.to_string_lossy().to_lowercase());
    INTERPRETERS.iter().find(|(e, _)| *e == ext).map(|(_, i)| *i)
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let data = std::fs::read(path)?;
    Ok(Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect())
}

/// Nombre sugerido para una tool a partir del archivo.
pub fn slug(file: &str) -> String {
    let stem = Path::new(file).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut out = String::new();
    for c in stem.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let s: String = out.trim_matches('-').chars().take(40).collect();
    if s.is_empty() { "tool".into() } else { s }
}

/// `tool.json` (las claves de la v1, para poder traer las tools de la v1 tal cual).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolMeta {
    #[serde(rename = "nombre")]
    pub name: String,
    #[serde(rename = "archivo")]
    pub file: String,
    #[serde(rename = "interprete")]
    pub interpreter: String,
    #[serde(rename = "descripcion", default)]
    pub description: String,
    #[serde(rename = "uso", default)]
    pub usage: String,
    /// `{ruta, sesiones, wis}`.
    #[serde(rename = "origen", default)]
    pub origin: Json,
    #[serde(default)]
    pub sha256: String,
    #[serde(rename = "creada", default)]
    pub created: String,
}

/// Una tool promovida, con su ruta y si el script sigue igual que cuando se promovió.
#[derive(Debug, Clone)]
pub struct Tool {
    pub meta: ToolMeta,
    pub path: PathBuf,
    pub ok: bool,
}

impl Tool {
    /// Para la UI (claves en inglés).
    pub fn to_json(&self) -> Json {
        let o = &self.meta.origin;
        json!({
            "name": self.meta.name,
            "file": self.meta.file,
            "interpreter": self.meta.interpreter,
            "description": self.meta.description,
            "usage": self.meta.usage,
            "origin": {
                "path": o["ruta"].as_str().unwrap_or(""),
                "sessions": o.get("sesiones").cloned().unwrap_or_else(|| json!([])),
                "items": o.get("wis").cloned().unwrap_or_else(|| json!([])),
            },
            "sha256": self.meta.sha256,
            "created": self.meta.created,
            "ok": self.ok,
            "path": self.path.display().to_string(),
        })
    }
}

/// Tools promovidas, por nombre de carpeta.
pub fn list(config_dir: &Path) -> Vec<Tool> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(tools_dir(config_dir))
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.join("tool.json").is_file()).collect())
        .unwrap_or_default();
    dirs.sort();
    dirs.iter()
        .filter_map(|d| {
            let meta: ToolMeta =
                serde_json::from_str(&std::fs::read_to_string(d.join("tool.json")).ok()?).ok()?;
            let path = d.join(&meta.file);
            let ok = !meta.file.is_empty()
                && path.is_file()
                && sha256_file(&path).is_ok_and(|h| h.eq_ignore_ascii_case(&meta.sha256));
            Some(Tool { meta, path, ok })
        })
        .collect()
}

pub fn get(config_dir: &Path, name: &str) -> Option<Tool> {
    list(config_dir).into_iter().find(|t| t.meta.name == name)
}

fn one_line(s: &str, max: usize) -> String {
    wb_core::py::split_ws(s).collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

fn write_meta(dir: &Path, meta: &ToolMeta) -> Result<()> {
    let tmp = dir.join("tool.json.tmp");
    let text = serde_json::to_string_pretty(meta).map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, dir.join("tool.json"))?;
    Ok(())
}

/// Copia `src` como la tool `name`. `origin` = de qué sesiones e items salió (`{sesiones, wis}`).
pub fn promote(
    config_dir: &Path,
    src: &Path,
    name: &str,
    description: &str,
    usage: &str,
    origin: Json,
    replace: bool,
) -> Result<Tool> {
    if !src.is_file() {
        return Err(err(
            ErrorKind::NotFound,
            "tools.error.missing",
            vec![("path", src.display().to_string())],
        ));
    }
    let Some(interpreter) = interpreter_of(src) else {
        let ext = src.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        return Err(err(ErrorKind::Invalid, "tools.error.unsupported", vec![("ext", ext)]));
    };
    if std::fs::metadata(src)?.len() > MAX_SCRIPT_BYTES {
        return Err(err(ErrorKind::Invalid, "tools.error.tooBig", vec![]));
    }
    let name = name.trim().to_lowercase();
    if !NAME_RE.is_match(&name) {
        return Err(err(ErrorKind::Invalid, "tools.error.badName", vec![]));
    }
    let dir = tools_dir(config_dir).join(&name);
    if dir.exists() && !replace {
        return Err(err(ErrorKind::Exists, "tools.error.exists", vec![("name", name)]));
    }
    // Antes de limpiar: el origen puede ser la copia de la propia tool.
    let data = std::fs::read(src)?;
    std::fs::create_dir_all(&dir)?;
    for old in std::fs::read_dir(&dir)?.flatten().map(|e| e.path()) {
        if old.is_file() && old.file_name().is_some_and(|n| n != "tool.json") {
            std::fs::remove_file(old)?;
        }
    }
    let file = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let target = dir.join(&file);
    std::fs::write(&target, &data)?;
    let mut origin = if origin.is_object() { origin } else { json!({}) };
    origin["ruta"] = json!(src.display().to_string());
    let meta = ToolMeta {
        name,
        file,
        interpreter: interpreter.into(),
        description: one_line(description, 500),
        usage: one_line(usage, 300),
        origin,
        sha256: sha256_file(&target)?,
        created: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
    };
    write_meta(&dir, &meta)?;
    Ok(Tool { meta, path: target, ok: true })
}

fn existing(config_dir: &Path, name: &str) -> Result<PathBuf> {
    let dir = tools_dir(config_dir).join(name);
    if !NAME_RE.is_match(name) || !dir.join("tool.json").is_file() {
        return Err(err(ErrorKind::NotFound, "tools.error.notFound", vec![("name", name.into())]));
    }
    Ok(dir)
}

pub fn update(config_dir: &Path, name: &str, description: &str, usage: &str) -> Result<Tool> {
    let dir = existing(config_dir, name)?;
    let mut meta: ToolMeta = serde_json::from_str(&std::fs::read_to_string(dir.join("tool.json"))?)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    meta.description = one_line(description, 500);
    meta.usage = one_line(usage, 300);
    write_meta(&dir, &meta)?;
    get(config_dir, name)
        .ok_or_else(|| err(ErrorKind::NotFound, "tools.error.notFound", vec![("name", name.into())]))
}

/// Se mueve a `tools/_removed/<nombre>-<fecha>` en vez de borrarla.
pub fn remove(config_dir: &Path, name: &str) -> Result<()> {
    let dir = existing(config_dir, name)?;
    let trash = tools_dir(config_dir).join(REMOVED_DIR);
    std::fs::create_dir_all(&trash)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    std::fs::rename(&dir, trash.join(format!("{name}-{stamp}")))?;
    Ok(())
}

static DOCSTRING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)^\s*(?:#[^\n]*\n\s*)*(?:"""|''')(.*?)(?:"""|''')"#).expect("regex"));
static ARGPARSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"add_argument\(\s*['"]([^'"]+)['"]"#).expect("regex"));
static PS_PARAM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)param\s*\((.*?)\)\s*\n").expect("regex"));
static PS_VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$(\w+)").expect("regex"));
static SYS_ARGV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"sys\.argv\[(\d+)\]").expect("regex"));

/// Descripción y uso sugeridos leyendo el script: docstring o comentario inicial, y los argumentos de
/// argparse / `sys.argv` / `param()` de PowerShell.
pub fn describe(path: &Path) -> (String, String) {
    let Ok(bytes) = std::fs::read(path) else { return (String::new(), String::new()) };
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_SCRIPT_BYTES as usize)]).to_string();
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut desc = String::new();
    if ext == "py"
        && let Some(c) = DOCSTRING.captures(&text)
    {
        desc = c[1].trim().to_string();
    }
    if desc.is_empty() {
        let mut lines: Vec<String> = vec![];
        for line in text.lines() {
            let s = line.trim();
            if s.starts_with("#!") || (s.is_empty() && lines.is_empty()) {
                continue;
            }
            if s.starts_with('#') || s.starts_with("//") || s.starts_with("REM ") || s.starts_with("::") {
                let s = s.strip_prefix("REM ").or_else(|| s.strip_prefix("::")).unwrap_or(s);
                lines.push(s.trim_start_matches(['#', '/', ' ']).trim().to_string());
            } else {
                break;
            }
        }
        desc = lines.into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    }
    let desc = one_line(&desc, 400);
    let mut args: Vec<String> = ARGPARSE.captures_iter(&text).map(|c| c[1].to_string()).collect();
    if args.is_empty()
        && ext == "ps1"
        && let Some(c) = PS_PARAM.captures(&text)
    {
        args = PS_VAR.captures_iter(&c[1]).map(|v| format!("-{}", &v[1])).collect();
    }
    if args.is_empty() {
        let mut n: Vec<u32> =
            SYS_ARGV.captures_iter(&text).filter_map(|c| c[1].parse().ok()).filter(|&i| i > 0).collect();
        n.sort_unstable();
        n.dedup();
        args = n.iter().map(|i| format!("<arg{i}>")).collect();
    }
    let pos = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|a| if a.starts_with('<') { a.clone() } else { format!("<{a}>") });
    let opt = args.iter().filter(|a| a.starts_with('-')).map(|a| format!("[{a} …]"));
    (desc, pos.chain(opt).collect::<Vec<_>>().join(" "))
}

/// Salida de una tool: si es larga, el final con la marca de recortada delante (como la v1).
fn decode(b: &[u8], cut: &str) -> String {
    let s = String::from_utf8_lossy(b);
    let n = s.chars().count();
    if n <= OUTPUT_MAX {
        return s.to_string();
    }
    let tail: String = s.chars().skip(n - OUTPUT_MAX).collect();
    format!("{cut}\n{tail}")
}

/// Corre una tool promovida en `cwd`. Falla si el script cambió desde que se promovió. `lang`: idioma de
/// las marcas que se suman a la salida (recortada, cortada por tiempo).
pub fn run(
    config_dir: &Path,
    name: &str,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
    lang: &str,
) -> Result<Json> {
    let tool = get(config_dir, name)
        .ok_or_else(|| err(ErrorKind::NotFound, "tools.error.notFound", vec![("name", name.into())]))?;
    if !tool.ok {
        return Err(err(ErrorKind::Permission, "tools.error.changed", vec![("name", name.into())]));
    }
    let mut argv = wb_platform::script_command(&tool.meta.interpreter, &tool.path).ok_or_else(|| {
        err(
            ErrorKind::NotFound,
            "tools.error.noInterpreter",
            vec![("interpreter", tool.meta.interpreter.clone())],
        )
    })?;
    argv.extend(args.iter().map(Into::into));
    let start = Instant::now();
    let out = wb_platform::run_captured(&argv, cwd, timeout)?;
    let cut = wb_i18n::t(lang, "mcp.tool.cut", &[]);
    let mut stderr = decode(&out.stderr, &cut);
    if out.timed_out {
        let secs = timeout.as_secs().to_string();
        stderr.push_str(&format!("\n{}", wb_i18n::t(lang, "mcp.tool.timeout", &[("seconds", &secs)])));
    }
    Ok(json!({
        "tool": name,
        "exitCode": out.code,
        "stdout": decode(&out.stdout, &cut),
        "stderr": stderr,
        "seconds": (start.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        "timedOut": out.timed_out,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-tools-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn slug_and_interpreter() {
        assert_eq!(slug("DIAG_WI123 Ver cosas.py"), "diag-wi123-ver-cosas");
        assert_eq!(slug("___.py"), "tool");
        assert_eq!(interpreter_of(Path::new("a/b.PS1")), Some("powershell"));
        assert_eq!(interpreter_of(Path::new("a/b.txt")), None);
    }

    #[test]
    fn describe_reads_docstring_and_args() {
        let d = tmp("describe");
        let py = d.join("x.py");
        std::fs::write(&py, "#!/usr/bin/env python\n\"\"\"Cuenta filas\n de una tabla.\"\"\"\nimport argparse\np.add_argument('tabla')\np.add_argument(\"--env\")\n").unwrap();
        assert_eq!(describe(&py), ("Cuenta filas de una tabla.".into(), "<tabla> [--env …]".into()));
        let ps = d.join("y.ps1");
        std::fs::write(
            &ps,
            "# Limpia la cache\n# del sitio\nparam(\n  [string]$Ruta,\n  [int]$Dias\n)\nWrite-Host 1\n",
        )
        .unwrap();
        assert_eq!(describe(&ps), ("Limpia la cache del sitio".into(), "[-Ruta …] [-Dias …]".into()));
        let argv = d.join("z.py");
        std::fs::write(&argv, "import sys\nprint(sys.argv[2], sys.argv[1], sys.argv[0])\n").unwrap();
        assert_eq!(describe(&argv).1, "<arg1> <arg2>");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn promote_freeze_update_remove() {
        let d = tmp("promote");
        let cfg = d.join("cfg");
        let src = d.join("count.bat");
        std::fs::write(&src, "@echo off\necho %1\n").unwrap();
        let bad = promote(&cfg, &src, "X Y", "d", "", json!({}), false).unwrap_err();
        assert_eq!(bad.code, "tools.error.badName");
        let t = promote(
            &cfg,
            &src,
            "count",
            "  Cuenta\n cosas ",
            "<x>",
            json!({"sesiones": ["s1"], "wis": ["wi1"]}),
            false,
        )
        .unwrap();
        assert!(t.ok);
        assert_eq!(t.meta.description, "Cuenta cosas");
        let raw: Json =
            serde_json::from_str(&std::fs::read_to_string(cfg.join("tools/count/tool.json")).unwrap())
                .unwrap();
        assert_eq!(raw["nombre"], "count");
        assert_eq!(raw["interprete"], "cmd");
        assert_eq!(raw["origen"]["wis"][0], "wi1");
        assert!(raw["origen"]["ruta"].as_str().unwrap().ends_with("count.bat"));
        assert_eq!(
            promote(&cfg, &src, "count", "d", "", json!({}), false).unwrap_err().code,
            "tools.error.exists"
        );
        // Cambiar la copia la congela.
        std::fs::write(&t.path, "@echo off\necho otro\n").unwrap();
        assert!(!get(&cfg, "count").unwrap().ok);
        assert_eq!(run(&cfg, "count", &[], &d, RUN_TIMEOUT, "es").unwrap_err().code, "tools.error.changed");
        // Promover de nuevo desde la copia acepta el cambio.
        let again = promote(&cfg, &t.path, "count", "d", "", t.meta.origin.clone(), true).unwrap();
        assert!(again.ok);
        let u = update(&cfg, "count", "nueva", "<y>").unwrap();
        assert_eq!((u.meta.description.as_str(), u.meta.usage.as_str()), ("nueva", "<y>"));
        assert_eq!(list(&cfg).len(), 1);
        remove(&cfg, "count").unwrap();
        assert!(list(&cfg).is_empty());
        assert!(cfg.join("tools").join(REMOVED_DIR).read_dir().unwrap().next().is_some());
        assert_eq!(remove(&cfg, "count").unwrap_err().code, "tools.error.notFound");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn long_output_keeps_the_tail_with_the_cut_mark() {
        assert_eq!(decode(b"corto", "CUT"), "corto");
        let long = "a".repeat(OUTPUT_MAX) + "fin";
        let out = decode(long.as_bytes(), "CUT");
        assert!(out.starts_with("CUT\n") && out.ends_with("fin"), "{}", &out[..10]);
        assert_eq!(out.chars().count(), OUTPUT_MAX + 4);
    }

    #[cfg(windows)]
    #[test]
    fn run_a_tool_in_a_folder() {
        let d = tmp("run");
        let cfg = d.join("cfg");
        let src = d.join("hello.bat");
        std::fs::write(&src, "@echo off\r\necho hola %1 & cd\r\nexit /b 2\r\n").unwrap();
        promote(&cfg, &src, "hello", "Saluda", "<nombre>", json!({}), false).unwrap();
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let out = run(&cfg, "hello", &["mundo".into()], &work, RUN_TIMEOUT, "es").unwrap();
        assert_eq!(out["exitCode"], 2);
        let stdout = out["stdout"].as_str().unwrap();
        assert!(stdout.contains("hola mundo"), "{stdout}");
        assert!(stdout.to_lowercase().contains("work"), "{stdout}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
