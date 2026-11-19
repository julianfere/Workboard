//! Métricas por sesión de agente (sesiones como evals).
//!
//! Fuentes, todas de lectura:
//! - `<config>/sessions/<id>.activity.jsonl`: pedidos del usuario, pedidos del agente (`Notification`) y
//!   launch/exit, de los hooks.
//! - transcript de Claude Code: modelo, tokens, herramientas, scripts escritos y corridos,
//!   interrupciones. Caché por fecha y tamaño en `<config>/stats_cache.json`.
//! - historial del estado de cada item: los backups del README (uno antes de cada escritura) más
//!   `<config>/state_log.jsonl` (cada cambio de estado). Dice si el item avanzó o retrocedió mientras la
//!   sesión trabajaba.
//!
//! Tokens desconocidos (agentes sin transcript) quedan en `None`, no en 0.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use wb_core::frontmatter::{self, Meta, Value};
use wb_core::{StateCategory, Workflow, Workspace};

use crate::agents;
use crate::registry::{Session, epoch, normcase};
use crate::timelog::{file_key, is_prompt, read_jsonl};
use crate::transcript;

/// Extensiones de lo que cuenta como script escrito por una sesión.
pub const SCRIPT_EXT: [&str; 6] = [".py", ".ps1", ".sh", ".mjs", ".cjs", ".bat"];
const SHELL_TOOLS: [&str; 2] = ["Bash", "PowerShell"];
const INTERRUPT: &str = "[Request interrupted by user";
const CACHE_VERSION: u64 = 1;

static RUN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[\w\-./\\:~]+\.(?:py|ps1|sh|mjs|cjs|bat)\b").expect("regex"));

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Written {
    pub path: String,
    pub ts: f64,
    pub chars: usize,
}

/// Resumen de un transcript de Claude Code.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptStats {
    pub models: BTreeMap<String, u64>,
    pub tokens: Option<Tokens>,
    pub tools: BTreeMap<String, u64>,
    pub prompts: u64,
    pub interrupts: u64,
    pub written: Vec<Written>,
    /// Corridas por nombre de archivo (en minúsculas).
    pub runs: BTreeMap<String, u64>,
    pub first: Option<f64>,
    pub last: Option<f64>,
}

fn text_of(content: &Json) -> String {
    match content {
        Json::String(s) => s.clone(),
        Json::Array(l) => l
            .iter()
            .filter(|c| c["type"] == "text")
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn path_key(p: &str) -> String {
    if cfg!(windows) { p.replace('/', "\\").to_lowercase() } else { p.to_string() }
}

/// Un mismo mensaje del asistente viene partido en varias líneas (una por bloque) con el mismo
/// `message.id` y el mismo `usage`: se cuenta una vez.
pub fn parse_transcript(path: &Path) -> TranscriptStats {
    let mut st = TranscriptStats::default();
    let mut tokens = Tokens::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut written: BTreeMap<String, Written> = BTreeMap::new();
    for d in read_jsonl(path) {
        if d["isSidechain"].as_bool().unwrap_or(false) {
            continue;
        }
        let ts = d["timestamp"].as_str().map_or(0.0, epoch);
        if ts > 0.0 {
            st.first = Some(st.first.map_or(ts, |f| f.min(ts)));
            st.last = Some(st.last.map_or(ts, |l| l.max(ts)));
        }
        let msg = &d["message"];
        if d["type"] == "user" {
            if is_prompt(&d) {
                if text_of(&msg["content"]).contains(INTERRUPT) {
                    st.interrupts += 1;
                } else {
                    st.prompts += 1;
                }
            }
            continue;
        }
        if d["type"] != "assistant" {
            continue;
        }
        if let Some(mid) = msg["id"].as_str()
            && seen.insert(mid.to_string())
        {
            let u = &msg["usage"];
            let n = |k: &str| u[k].as_u64().unwrap_or(0);
            tokens.input += n("input_tokens");
            tokens.output += n("output_tokens");
            tokens.cache_read += n("cache_read_input_tokens");
            tokens.cache_write += n("cache_creation_input_tokens");
            if let Some(m) = msg["model"].as_str().filter(|m| !m.is_empty() && !m.starts_with('<')) {
                *st.models.entry(m.to_string()).or_default() += 1;
            }
        }
        for c in msg["content"].as_array().into_iter().flatten().filter(|c| c["type"] == "tool_use") {
            let name = c["name"].as_str().unwrap_or("");
            let inp = &c["input"];
            *st.tools.entry(name.to_string()).or_default() += 1;
            if name == "Write" {
                let fp = inp["file_path"].as_str().unwrap_or("");
                if SCRIPT_EXT.iter().any(|e| fp.to_lowercase().ends_with(e)) {
                    let chars = inp["content"].as_str().map_or(0, |s| s.chars().count());
                    written.insert(path_key(fp), Written { path: fp.to_string(), ts, chars });
                }
            } else if SHELL_TOOLS.contains(&name) {
                let cmd = inp["command"].as_str().unwrap_or("");
                let found: BTreeSet<String> = RUN_RE
                    .find_iter(cmd)
                    .map(|m| m.as_str().replace('\\', "/").rsplit('/').next().unwrap_or("").to_lowercase())
                    .collect();
                for k in found {
                    *st.runs.entry(k).or_default() += 1;
                }
            }
        }
    }
    st.tokens = (!seen.is_empty()).then_some(tokens);
    st.written = written.into_values().collect();
    st
}

#[derive(Default, Serialize, Deserialize)]
struct CacheData {
    v: u64,
    t: BTreeMap<String, CacheEntry>,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    key: String,
    data: TranscriptStats,
}

struct Cache {
    path: PathBuf,
    data: CacheData,
    dirty: bool,
}

impl Cache {
    fn load(path: PathBuf) -> Self {
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<CacheData>(&t).ok())
            .filter(|d| d.v == CACHE_VERSION)
            .unwrap_or(CacheData { v: CACHE_VERSION, t: BTreeMap::new() });
        Self { path, data, dirty: false }
    }

    fn stats(&mut self, sid: &str, path: &Path) -> TranscriptStats {
        let key = file_key(path).unwrap_or_default();
        if let Some(e) = self.data.t.get(sid).filter(|e| e.key == key) {
            return e.data.clone();
        }
        let data = parse_transcript(path);
        self.data.t.insert(sid.to_string(), CacheEntry { key, data: data.clone() });
        self.dirty = true;
        data
    }

    fn save(mut self, live: &BTreeSet<&str>) {
        if !self.dirty {
            return;
        }
        self.data.t.retain(|k, _| live.contains(k.as_str()));
        let _ = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(self.path.parent().unwrap_or(Path::new(".")))?;
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_string(&self.data).map_err(std::io::Error::other)?)?;
            std::fs::rename(&tmp, &self.path)
        })();
    }
}

// ---------- historial de estados ----------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub ts: f64,
    pub from: String,
    pub to: String,
    /// Sesión que hizo el cambio, si se sabe (el log lo sabe cuando vino del MCP de una sesión).
    pub session: Option<String>,
}

fn status_of(meta: &Meta, wf: &Workflow) -> String {
    meta.get(&wf.fields.status).and_then(Value::as_text).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Filas de `state_log.jsonl` de una raíz.
pub fn read_state_log(config_dir: &Path, root: &Path) -> Vec<Json> {
    let key = normcase(root);
    read_jsonl(&config_dir.join("state_log.jsonl"))
        .into_iter()
        .filter(|e| e["root"] == key.as_str())
        .collect()
}

/// Cambios de estado del item. Cada backup es el README *antes* de una escritura: el estado después de
/// esa escritura es el del backup siguiente (o el README actual). Backups y log cuentan el mismo cambio:
/// gana el del log (sabe la sesión), con 2 s de tolerancia.
pub fn state_history(ws: &Workspace, name: &str, state_log: &[Json]) -> Vec<Transition> {
    let wf = ws.workflow();
    let mut versions = ws.list_versions(name).unwrap_or_default();
    versions.sort_by(|a, b| a.id.cmp(&b.id));
    let snaps: Vec<(f64, String)> = versions
        .iter()
        .filter_map(|v| {
            let text = ws.read_version(name, &v.id).ok()?;
            Some((v.ts, status_of(&frontmatter::split(&text).0, wf)))
        })
        .collect();
    let current = ws.get_meta(name).ok().map(|m| status_of(&m, wf));
    let mut out: Vec<Transition> = vec![];
    for (i, (ts, before)) in snaps.iter().enumerate() {
        let after = snaps.get(i + 1).map(|s| s.1.clone()).or_else(|| current.clone());
        if let Some(after) = after
            && &after != before
        {
            out.push(Transition { ts: *ts, from: before.clone(), to: after, session: None });
        }
    }
    let logged: Vec<Transition> = state_log
        .iter()
        .filter(|e| e["wi"] == name && e["de"] != e["a"])
        .map(|e| Transition {
            ts: e["ts"].as_f64().unwrap_or(0.0),
            from: e["de"].as_str().unwrap_or("").into(),
            to: e["a"].as_str().unwrap_or("").into(),
            session: e["session"].as_str().map(String::from),
        })
        .collect();
    out.retain(|x| !logged.iter().any(|y| (x.ts - y.ts).abs() <= 2.0 && x.from == y.from && x.to == y.to));
    out.extend(logged);
    out.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap_or(std::cmp::Ordering::Equal));
    out
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub start: Option<String>,
    pub end: Option<String>,
    /// Pasos que avanzó (posición en el flujo; negativo = terminó más atrás).
    pub advance: i64,
    /// Veces que volvió a un estado anterior (retrabajo).
    pub setbacks: u64,
    pub transitions: Vec<Transition>,
}

/// Posición de un estado para medir avance: descartado o desconocido no avanzan ni retroceden.
fn rank(wf: &Workflow, id: &str) -> Option<i64> {
    let s = wf.state(id)?;
    if matches!(s.category, StateCategory::Cancelled | StateCategory::Unknown) {
        return None;
    }
    wf.state_rank(id).map(|r| r as i64)
}

pub fn outcome(wf: &Workflow, transitions: Vec<Transition>) -> Outcome {
    let setbacks = transitions
        .iter()
        .filter(|t| matches!((rank(wf, &t.from), rank(wf, &t.to)), (Some(a), Some(b)) if b < a))
        .count() as u64;
    let start = transitions.first().map(|t| t.from.clone());
    let end = transitions.last().map(|t| t.to.clone());
    let advance = match (start.as_deref().and_then(|s| rank(wf, s)), end.as_deref().and_then(|s| rank(wf, s)))
    {
        (Some(a), Some(b)) => b - a,
        _ => 0,
    };
    Outcome { start, end, advance, setbacks, transitions }
}

// ---------- reporte ----------

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Scripts {
    pub written: Vec<Written>,
    pub runs: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    pub id: String,
    pub name: String,
    pub agent: String,
    pub mode: String,
    pub plan: bool,
    pub model: Option<String>,
    pub state: crate::registry::State,
    pub items: Vec<String>,
    pub fork_of: Option<String>,
    pub from: Option<f64>,
    pub to: Option<f64>,
    pub models: BTreeMap<String, u64>,
    pub tokens: Option<Tokens>,
    pub prompts: u64,
    pub interrupts: u64,
    /// Veces que el agente pidió permiso o una respuesta.
    pub asks: u64,
    pub tools: BTreeMap<String, u64>,
    pub scripts: Scripts,
    /// Lo que pasó con cada item mientras la sesión trabajaba.
    pub outcome: BTreeMap<String, Outcome>,
    /// Transcript usado (si hay).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
}

/// Una fila por sesión de la raíz: uso, intervención, herramientas, scripts y avance de sus items.
pub fn report(
    ws: &Workspace,
    sessions: &[Session],
    config_dir: &Path,
    projects: Option<&Path>,
) -> Vec<SessionStats> {
    let mut cache = Cache::load(config_dir.join("stats_cache.json"));
    let sess_dir = config_dir.join("sessions");
    let slog = read_state_log(config_dir, ws.root());
    let mut hist: HashMap<String, Vec<Transition>> = HashMap::new();
    let mut rows = vec![];
    for s in sessions {
        let events = read_jsonl(&sess_dir.join(format!("{}.activity.jsonl", s.id)));
        let def = agents::agent(&s.agent);
        let tr = if def.is_some_and(|d| d.transcript) {
            projects.and_then(|p| transcript::find(p, &s.id))
        } else {
            None
        };
        let t = tr.as_ref().map(|p| cache.stats(&s.id, p));
        let mut stamps: Vec<f64> = events.iter().filter_map(|e| e["ts"].as_f64()).collect();
        if let Some(t) = &t {
            stamps.extend(t.first.iter().chain(t.last.iter()));
        }
        let created = Some(epoch(&s.created)).filter(|x| *x > 0.0);
        let from = stamps.iter().copied().chain(created).reduce(f64::min);
        let to = stamps
            .iter()
            .copied()
            .reduce(f64::max)
            .or_else(|| Some(epoch(&s.last_activity)).filter(|x| *x > 0.0));
        let mut out_items = BTreeMap::new();
        for w in &s.items {
            let h = hist.entry(w.clone()).or_insert_with(|| state_history(ws, w, &slog));
            // Dentro de la ventana de la sesión; si el log sabe que lo hizo otra sesión, no cuenta.
            let during: Vec<Transition> = match (from, to) {
                (Some(a), Some(b)) => h
                    .iter()
                    .filter(|x| a - 60.0 <= x.ts && x.ts <= b + 60.0)
                    .filter(|x| x.session.as_deref().is_none_or(|sid| sid == s.id))
                    .cloned()
                    .collect(),
                _ => vec![],
            };
            out_items.insert(w.clone(), outcome(ws.workflow(), during));
        }
        let count = |ev: &str| events.iter().filter(|e| e["ev"] == ev).count() as u64;
        let asks =
            events.iter().filter(|e| e["ev"] == "Notification" && e["st"] == "needsYou").count() as u64;
        let t = t.unwrap_or_default();
        let has_t = tr.is_some();
        rows.push(SessionStats {
            id: s.id.clone(),
            name: s.name.clone(),
            agent: s.agent.clone(),
            mode: s.mode.clone(),
            plan: s.plan,
            model: s.model.clone(),
            state: s.state,
            items: s.items.clone(),
            fork_of: s.fork_of.clone(),
            from,
            to,
            models: t.models,
            tokens: t.tokens,
            prompts: if has_t { t.prompts } else { count("UserPromptSubmit") },
            interrupts: t.interrupts,
            asks,
            tools: t.tools,
            scripts: Scripts { written: t.written, runs: t.runs },
            outcome: out_items,
            transcript: tr.map(|p| p.display().to_string()),
        });
    }
    cache.save(&sessions.iter().map(|s| s.id.as_str()).collect());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn transcript_counts_each_message_once() {
        let dir = std::env::temp_dir().join(format!("wb-st-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.jsonl");
        let usage = json!({"input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 100, "cache_creation_input_tokens": 1});
        let lines = [
            json!({"type": "user", "timestamp": "2026-11-19T10:00:00Z", "message": {"content": "hola"}}),
            json!({"type": "assistant", "timestamp": "2026-11-19T10:00:05Z", "message": {"id": "m1", "model": "claude-x", "usage": usage,
                "content": [{"type": "tool_use", "name": "Write", "input": {"file_path": "C:/w/wi1/DIAG_x.py", "content": "print(1)"}}]}}),
            json!({"type": "assistant", "timestamp": "2026-11-19T10:00:06Z", "message": {"id": "m1", "model": "claude-x", "usage": usage,
                "content": [{"type": "tool_use", "name": "Bash", "input": {"command": "py C:/w/wi1/DIAG_x.py && py DIAG_x.py --a"}}]}}),
            json!({"type": "assistant", "timestamp": "2026-11-19T10:00:07Z", "isSidechain": true, "message": {"id": "m2", "usage": usage, "content": []}}),
            json!({"type": "user", "timestamp": "2026-11-19T10:01:00Z", "message": {"content": [{"type": "text", "text": "[Request interrupted by user]"}]}}),
            json!({"type": "assistant", "timestamp": "2026-11-19T10:02:00Z", "message": {"id": "m3", "model": "<synthetic>", "usage": {"output_tokens": 1}, "content": []}}),
        ];
        std::fs::write(&p, lines.iter().map(Json::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let st = parse_transcript(&p);
        assert_eq!(st.tokens, Some(Tokens { input: 10, output: 6, cache_read: 100, cache_write: 1 }));
        assert_eq!(st.models.get("claude-x"), Some(&1));
        assert_eq!(st.models.len(), 1);
        assert_eq!((st.prompts, st.interrupts), (1, 1));
        assert_eq!(st.tools.get("Write"), Some(&1));
        assert_eq!(st.written.len(), 1);
        assert_eq!(st.written[0].chars, 8);
        // Dos menciones del mismo script en un comando cuentan una corrida.
        assert_eq!(st.runs.get("diag_x.py"), Some(&1));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn outcome_measures_advance_and_setbacks() {
        let wf = Workflow::preset("classic-es").unwrap();
        let tr =
            |from: &str, to: &str| Transition { ts: 0.0, from: from.into(), to: to.into(), session: None };
        let o = outcome(
            &wf,
            vec![tr("plan", "en-curso"), tr("en-curso", "qa"), tr("qa", "en-curso"), tr("en-curso", "prod")],
        );
        assert_eq!((o.start.as_deref(), o.end.as_deref()), (Some("plan"), Some("prod")));
        assert_eq!(o.setbacks, 1);
        assert!(o.advance > 0);
        let none = outcome(&wf, vec![tr("en-curso", "descartado")]);
        assert_eq!((none.advance, none.setbacks), (0, 0));
        assert_eq!(outcome(&wf, vec![]).start, None);
    }
}
