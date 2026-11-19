//! Lo que se lee del transcript de Claude Code (`~/.claude/projects/*/<id>.jsonl`): el peso de una
//! sesión y cuándo editó código o el README por última vez.
//!
//! Cada llamada al modelo relee todo el contexto, así que lo que cuesta es el tamaño del contexto de la
//! última respuesta (no la cantidad de mensajes; un `/compact` lo baja).

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value as Json;

use crate::registry::epoch;

pub const LONG_CTX: u64 = 180_000;
pub const HEAVY_CTX: u64 = 350_000;
/// La última respuesta está al final del transcript.
const TAIL_BYTES: u64 = 1024 * 1024;
/// Herramientas que editan archivos.
pub const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
/// Sin actividad hace más de esto = retomar otro día: conviene sesión nueva.
pub const IDLE_HANDOFF: f64 = 3.0 * 3600.0;

/// Variable que cambia la carpeta de transcripts (pruebas, desarrollo).
pub const PROJECTS_ENV: &str = "WORKBOARD_CLAUDE_PROJECTS";

/// Carpeta de transcripts de Claude Code.
pub fn projects_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(PROJECTS_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join(".claude").join("projects"))
}

/// Transcript de una sesión de Claude Code, si existe.
pub fn find(projects: &Path, sid: &str) -> Option<PathBuf> {
    let name = format!("{sid}.jsonl");
    std::fs::read_dir(projects).ok()?.flatten().map(|e| e.path().join(&name)).find(|p| p.is_file())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailInfo {
    /// Tokens de contexto de la última respuesta (entrada + caché leída + caché escrita).
    pub ctx: u64,
    pub last_ts: f64,
    /// Última edición de código y del README (epoch).
    pub last_edit: f64,
    pub last_readme: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum EditKind {
    Code,
    Readme,
}

/// Respuesta del asistente (no de un subagente) con Edit/Write: ¿tocó código o solo el README?
fn edit_kind(d: &Json, readme: &str) -> Option<EditKind> {
    if d["type"] != "assistant" || d["isSidechain"].as_bool().unwrap_or(false) {
        return None;
    }
    let paths: Vec<String> = d["message"]["content"]
        .as_array()?
        .iter()
        .filter(|c| c["type"] == "tool_use" && c["name"].as_str().is_some_and(|n| EDIT_TOOLS.contains(&n)))
        .map(|c| {
            let inp = &c["input"];
            inp["file_path"].as_str().or_else(|| inp["notebook_path"].as_str()).unwrap_or("").to_string()
        })
        .collect();
    if paths.is_empty() {
        return None;
    }
    let is_readme = |p: &String| {
        Path::new(p).file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(readme))
    };
    Some(if paths.iter().all(is_readme) { EditKind::Readme } else { EditKind::Code })
}

fn read_from(path: &Path, start: u64) -> Vec<String> {
    let mut text = Vec::new();
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    if f.seek(SeekFrom::Start(start)).is_err() || f.read_to_end(&mut text).is_err() {
        return vec![];
    }
    String::from_utf8_lossy(&text).lines().map(String::from).collect()
}

/// Del final del transcript: contexto de la última respuesta, hora del último mensaje y de la última
/// edición de código y del README. Si las ediciones no están en el final, recorre el archivo entero.
pub fn tail_info(path: &Path, size: u64, readme: &str) -> TailInfo {
    let mut info = TailInfo::default();
    let tail = read_from(path, size.saturating_sub(TAIL_BYTES));
    for line in tail.iter().rev() {
        if info.ctx > 0 && info.last_ts > 0.0 && info.last_edit > 0.0 && info.last_readme > 0.0 {
            break;
        }
        let Ok(d) = serde_json::from_str::<Json>(line) else { continue };
        let ts = d["timestamp"].as_str().map_or(0.0, epoch);
        if info.last_ts == 0.0 && ts > 0.0 {
            info.last_ts = ts;
        }
        if info.ctx == 0 && d["type"] == "assistant" && !d["isSidechain"].as_bool().unwrap_or(false) {
            let u = &d["message"]["usage"];
            if u.is_object() {
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                info.ctx =
                    n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
            }
        }
        match edit_kind(&d, readme) {
            Some(EditKind::Code) if info.last_edit == 0.0 => info.last_edit = ts,
            Some(EditKind::Readme) if info.last_readme == 0.0 => info.last_readme = ts,
            _ => {}
        }
    }
    if size > TAIL_BYTES && (info.last_edit == 0.0 || info.last_readme == 0.0) {
        let all = read_from(path, 0);
        let head = &all[..all.len().saturating_sub(tail.len())];
        let (mut edit, mut readme_ts) = (0.0f64, 0.0f64);
        for line in head.iter().filter(|l| l.contains("\"tool_use\"")) {
            let Ok(d) = serde_json::from_str::<Json>(line) else { continue };
            let ts = d["timestamp"].as_str().map_or(0.0, epoch);
            match edit_kind(&d, readme) {
                Some(EditKind::Code) => edit = edit.max(ts),
                Some(EditKind::Readme) => readme_ts = readme_ts.max(ts),
                None => {}
            }
        }
        if info.last_edit == 0.0 {
            info.last_edit = edit;
        }
        if info.last_readme == 0.0 {
            info.last_readme = readme_ts;
        }
    }
    info
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Normal,
    Long,
    Heavy,
}

pub fn level(ctx: u64) -> Level {
    if ctx >= HEAVY_CTX {
        Level::Heavy
    } else if ctx >= LONG_CTX {
        Level::Long
    } else {
        Level::Normal
    }
}

/// Salida recomendada para una sesión larga.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    /// `new` | `handoff` | `compact`.
    pub action: &'static str,
    /// Código del motivo (lo traduce la UI) y sus parámetros.
    pub reason: &'static str,
    /// Hubo código que el README no refleja (si no, lo trabajado solo está en la conversación).
    pub code: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours: Option<u64>,
}

/// Cuál de las tres salidas conviene: lo que ya quedó en el README (resumen, pendientes, estado) no se
/// pierde al cortar; lo que solo está en la conversación, sí.
/// - nueva: el README se actualizó después del último cambio de código;
/// - traspaso: hay código sin reflejar y se cerró una etapa (cambió un estado) o se está retomando;
/// - compactar: hay trabajo en curso que solo está en la conversación.
pub fn recommend(
    info: &TailInfo,
    last_readme_event: f64,
    state_changed: bool,
    last_activity: f64,
    now: f64,
) -> Recommendation {
    let saved = last_readme_event.max(info.last_readme);
    let code = info.last_edit > 0.0;
    if saved > 0.0 && saved >= info.last_edit {
        return Recommendation { action: "new", reason: "session.rec.saved", code, hours: None };
    }
    let idle = now - info.last_ts.max(last_activity);
    if state_changed {
        return Recommendation { action: "handoff", reason: "session.rec.stateChanged", code, hours: None };
    }
    if idle >= IDLE_HANDOFF {
        let hours = Some((idle / 3600.0) as u64);
        return Recommendation { action: "handoff", reason: "session.rec.idle", code, hours };
    }
    Recommendation { action: "compact", reason: "session.rec.inProgress", code, hours: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ts: &str, kind: &str, file: &str, ctx: u64) -> String {
        serde_json::json!({
            "type": kind, "timestamp": ts,
            "message": {
                "usage": {"input_tokens": ctx, "cache_read_input_tokens": 0},
                "content": if file.is_empty() { serde_json::json!([]) } else {
                    serde_json::json!([{"type": "tool_use", "name": "Edit", "input": {"file_path": file}}])
                }
            }
        })
        .to_string()
    }

    #[test]
    fn tail_and_recommendation() {
        let dir = std::env::temp_dir().join(format!("wb-tr-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("proj")).unwrap();
        let path = dir.join("proj").join("s1.jsonl");
        let text = [
            line("2026-11-19T10:00:00Z", "assistant", "C:/r/src/a.rs", 1000),
            line("2026-11-19T10:05:00Z", "assistant", "C:/r/_work/wi1/README.md", 2000),
            line("2026-11-19T10:06:00Z", "assistant", "", 200_000),
            line("2026-11-19T10:07:00Z", "user", "", 0),
        ]
        .join("\n");
        std::fs::write(&path, &text).unwrap();
        assert_eq!(find(&dir, "s1"), Some(path.clone()));
        let info = tail_info(&path, text.len() as u64, "README.md");
        assert_eq!(info.ctx, 200_000);
        assert_eq!(level(info.ctx), Level::Long);
        assert!(info.last_readme > info.last_edit && info.last_edit > 0.0);
        assert_eq!(info.last_ts, epoch("2026-11-19T10:07:00Z"));
        let now = info.last_ts + 60.0;
        assert_eq!(recommend(&info, 0.0, false, 0.0, now).action, "new");
        let unsaved = TailInfo { last_readme: 0.0, ..info };
        assert_eq!(recommend(&unsaved, 0.0, false, 0.0, now).action, "compact");
        assert_eq!(recommend(&unsaved, 0.0, true, 0.0, now).action, "handoff");
        let late = recommend(&unsaved, 0.0, false, 0.0, now + 4.0 * 3600.0);
        assert_eq!((late.action, late.hours), ("handoff", Some(4)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
