//! Tiempo trabajado por item a partir de las sesiones de agentes.
//!
//! Dos fuentes, de más a menos exacta:
//! - registro: `<config>/sessions/<id>.activity.jsonl` (hooks del CLI + lanzamiento y fin del proceso).
//!   Tiempo activo = desde que el usuario manda un mensaje hasta que el agente termina (`Stop`) o pide
//!   algo (`Notification`); cada evento lleva los items que tenía la sesión en ese momento.
//! - transcript de Claude Code, para lo anterior al registro. Turno = mensaje del usuario → última
//!   respuesta antes del siguiente; los silencios largos dentro de un turno (esperando un permiso) se
//!   recortan. Se reparte entre los items actuales de la sesión.
//!
//! Agentes sin hooks: solo se sabe cuánto estuvo abierta (launch → exit). Todo en segundos y por día
//! local, para que la UI lo sume por período. El tiempo de un intervalo se divide en partes iguales entre
//! los items de la sesión en ese momento.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::agents;
use crate::registry::{Session, epoch};
use crate::transcript;

/// Un hueco mayor dentro de un turno (permiso, pausa) cuenta hasta acá.
pub const IDLE_CAP: f64 = 10.0 * 60.0;
/// Intervalo sin cierre (la app se cerró de golpe): como mucho esto.
pub const OPEN_CAP: f64 = 30.0 * 60.0;
const ACTIVE_START: [&str; 2] = ["UserPromptSubmit", "resume"];
const ACTIVE_END: [&str; 5] = ["Stop", "Notification", "SessionEnd", "exit", "launch"];
/// Cierres "buenos" de un turno (puede durar horas); los demás cuentan como mucho `OPEN_CAP`.
const CLEAN_END: [&str; 4] = ["Stop", "Notification", "SessionEnd", "exit"];

/// Líneas JSON de un archivo (las rotas se saltean).
pub fn read_jsonl(path: &Path) -> Vec<Json> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Json>(l).ok())
        .filter(Json::is_object)
        .collect()
}

/// Día local `AAAA-MM-DD` de un epoch.
pub fn day(ts: f64) -> String {
    chrono::Local
        .timestamp_opt(ts.floor() as i64, 0)
        .earliest()
        .map_or_else(String::new, |d| d.format("%Y-%m-%d").to_string())
}

/// Items de una fila de actividad (`items`; la v1 usaba `wis`).
fn row_items(e: &Json) -> Option<Vec<String>> {
    let v = e.get("items").or_else(|| e.get("wis"))?;
    Some(v.as_array()?.iter().filter_map(|x| x.as_str().map(String::from)).collect())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    pub start: f64,
    pub secs: f64,
    pub items: Vec<String>,
}

#[derive(Debug, Default)]
pub struct LogIntervals {
    pub active: Vec<Interval>,
    pub open: Vec<Interval>,
    /// `(ts, {viejo: nuevo})`.
    pub renames: Vec<(f64, BTreeMap<String, String>)>,
}

/// Intervalos del registro de actividad. `running` = el proceso sigue vivo: lo abierto cuenta hasta `now`.
pub fn log_intervals(events: &[Json], running: bool, now: f64) -> LogIntervals {
    let mut out = LogIntervals::default();
    let mut rows: Vec<&Json> = events.iter().filter(|e| e["ts"].is_number()).collect();
    rows.sort_by(|a, b| a["ts"].as_f64().partial_cmp(&b["ts"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
    let mut start: Option<(f64, Vec<String>)> = None;
    let mut launch: Option<(f64, Vec<String>)> = None;
    let mut items: Vec<String> = vec![];
    let mut last: Option<f64> = None;
    for e in rows {
        let ts = e["ts"].as_f64().unwrap_or(0.0);
        let ev = e["ev"].as_str().unwrap_or("");
        if ev == "rename" {
            let map = e["map"]
                .as_object()
                .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
                .unwrap_or_default();
            out.renames.push((ts, map));
            continue;
        }
        if let Some(i) = row_items(e) {
            items = i;
        }
        if (ACTIVE_END.contains(&ev) || ev == "SessionStart")
            && let Some((s, w)) = start.take()
        {
            // Un launch o SessionStart con un turno abierto = se perdió el cierre.
            let dur = if CLEAN_END.contains(&ev) { ts - s } else { (ts - s).min(OPEN_CAP) };
            out.active.push(Interval { start: s, secs: dur, items: w });
        }
        if ACTIVE_START.contains(&ev) && start.is_none() {
            start = Some((ts, items.clone()));
        }
        if (ev == "launch" || ev == "exit")
            && let Some((l, w)) = launch.take()
        {
            // Un launch sin exit (la app se cortó): estuvo abierta hasta su último evento.
            let end = if ev == "exit" { ts } else { last.unwrap_or(l) };
            out.open.push(Interval { start: l, secs: (end - l).max(0.0), items: w });
        }
        if ev == "launch" {
            launch = Some((ts, items.clone()));
        }
        last = Some(ts);
    }
    let end = if running { now } else { last.unwrap_or(now) };
    if let Some((s, w)) = start {
        out.active.push(Interval { start: s, secs: (end - s).min(OPEN_CAP), items: w });
    }
    if let Some((l, w)) = launch {
        out.open.push(Interval { start: l, secs: (end - l).max(0.0), items: w });
    }
    out
}

// ---------- transcript de Claude Code ----------

/// ¿Es un mensaje escrito por el usuario (no una respuesta de herramienta ni un mensaje del sistema)?
pub fn is_prompt(d: &Json) -> bool {
    if d["type"] != "user"
        || d["isMeta"].as_bool().unwrap_or(false)
        || d["isSidechain"].as_bool().unwrap_or(false)
    {
        return false;
    }
    match &d["message"]["content"] {
        Json::String(s) => !s.trim().is_empty(),
        Json::Array(l) => {
            l.iter().any(|x| x["type"] == "text") && !l.iter().any(|x| x["type"] == "tool_result")
        }
        _ => false,
    }
}

/// `(inicio, segundos)` por turno: del mensaje del usuario a la última respuesta antes del siguiente.
pub fn transcript_turns(path: &Path) -> Vec<(f64, f64)> {
    let mut turns = vec![];
    let mut cur: Option<f64> = None;
    let (mut last, mut acc) = (0.0f64, 0.0f64);
    for d in read_jsonl(path) {
        let kind = d["type"].as_str().unwrap_or("");
        if !(kind == "user" || kind == "assistant") || d["isSidechain"].as_bool().unwrap_or(false) {
            continue;
        }
        let ts = d["timestamp"].as_str().map_or(0.0, epoch);
        if ts == 0.0 {
            continue;
        }
        if is_prompt(&d) {
            if let Some(c) = cur {
                turns.push((c, acc));
            }
            (cur, last, acc) = (Some(ts), ts, 0.0);
        } else if cur.is_some() && ts >= last {
            acc += (ts - last).min(IDLE_CAP);
            last = ts;
        }
    }
    if let Some(c) = cur {
        turns.push((c, acc));
    }
    turns
}

/// Clave de caché de un archivo: fecha de modificación y tamaño.
pub fn file_key(path: &Path) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
    Some(format!("{mtime:.3}:{}", m.len()))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheEntry {
    key: String,
    turns: Vec<(f64, f64)>,
}

/// Turnos por transcript (`<config>/time_cache.json`), recalculados solo si el archivo cambió.
struct Cache {
    path: PathBuf,
    data: BTreeMap<String, CacheEntry>,
    dirty: bool,
}

impl Cache {
    fn load(path: PathBuf) -> Self {
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self { path, data, dirty: false }
    }

    fn turns(&mut self, sid: &str, path: &Path) -> Vec<(f64, f64)> {
        let key = file_key(path).unwrap_or_default();
        if let Some(e) = self.data.get(sid).filter(|e| e.key == key) {
            return e.turns.clone();
        }
        let round = |x: f64| (x * 10.0).round() / 10.0;
        let turns: Vec<(f64, f64)> =
            transcript_turns(path).into_iter().map(|(a, s)| (round(a), round(s))).collect();
        self.data.insert(sid.to_string(), CacheEntry { key, turns: turns.clone() });
        self.dirty = true;
        turns
    }

    fn save(mut self, live: &BTreeSet<&str>) {
        if !self.dirty {
            return;
        }
        self.data.retain(|k, _| live.contains(k.as_str()));
        let _ = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(self.path.parent().unwrap_or(Path::new(".")))?;
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_string(&self.data).map_err(std::io::Error::other)?)?;
            std::fs::rename(&tmp, &self.path)
        })();
    }
}

// ---------- reporte ----------

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemTime {
    /// Segundos activos por día local.
    pub days: BTreeMap<String, u64>,
    /// Segundos con la sesión abierta por día (agentes sin hooks).
    pub open: BTreeMap<String, u64>,
    /// Segundos activos por sesión.
    pub sessions: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionTime {
    pub id: String,
    pub name: String,
    pub agent: String,
    pub state: crate::registry::State,
    pub items: Vec<String>,
    pub active: u64,
    pub open: u64,
    /// `log`, `transcript`, `log+transcript` o "" (sin tiempo).
    pub source: String,
    pub from: Option<f64>,
    pub to: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimeReport {
    pub by_item: BTreeMap<String, ItemTime>,
    pub sessions: Vec<SessionTime>,
}

/// Sigue los renombres (agrupar, renombrar) hasta el nombre actual; un item dentro de un grupo
/// renombrado también sigue a su grupo.
fn resolve(name: &str, renames: &BTreeMap<String, String>) -> String {
    let mut cur = name.to_string();
    for _ in 0..32 {
        let next = crate::moved_name(renames, &cur);
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

/// Segundos por día (activos), por día (abierta) y por sesión de un item, antes de redondear.
type Acc = (BTreeMap<String, f64>, BTreeMap<String, f64>, BTreeMap<String, f64>);

/// Tiempo de las sesiones (`(sesión, ¿corre?)`) por item y por día.
pub fn report(
    sessions: &[(Session, bool)],
    config_dir: &Path,
    projects: Option<&Path>,
    now: f64,
) -> TimeReport {
    let mut cache = Cache::load(config_dir.join("time_cache.json"));
    let sess_dir = config_dir.join("sessions");
    let mut out = TimeReport::default();
    let mut renames: Vec<(f64, BTreeMap<String, String>)> = vec![];
    // (items, inicio, segundos, sesión, ¿activo?)
    let mut pending: Vec<(Vec<String>, f64, f64, String, bool)> = vec![];
    for (s, running) in sessions {
        let events = read_jsonl(&sess_dir.join(format!("{}.activity.jsonl", s.id)));
        let mut log = log_intervals(&events, *running, now);
        renames.append(&mut log.renames);
        let first_log = events.iter().filter_map(|e| e["ts"].as_f64()).reduce(f64::min);
        let mut source = if events.is_empty() { "" } else { "log" }.to_string();
        let def = agents::agent(&s.agent);
        if def.is_some_and(|d| d.transcript)
            && let Some(tr) = projects.and_then(|p| transcript::find(p, &s.id))
        {
            let created = epoch(&s.created);
            let old: Vec<Interval> = cache
                .turns(&s.id, &tr)
                .into_iter()
                .filter(|(a, _)| *a >= created - 60.0 && first_log.is_none_or(|f| *a < f))
                .map(|(a, secs)| Interval { start: a, secs, items: s.items.clone() })
                .collect();
            if !old.is_empty() {
                source = if events.is_empty() { "transcript" } else { "log+transcript" }.into();
                log.active.splice(0..0, old);
            }
        }
        let has_active = !log.active.is_empty();
        for i in &log.active {
            let items = if i.items.is_empty() { s.items.clone() } else { i.items.clone() };
            pending.push((items, i.start, i.secs, s.id.clone(), true));
        }
        for i in &log.open {
            let items = if i.items.is_empty() { s.items.clone() } else { i.items.clone() };
            pending.push((items, i.start, i.secs, s.id.clone(), false));
        }
        let tot = |v: &[Interval]| v.iter().map(|i| i.secs).sum::<f64>().round() as u64;
        let (active, open) = (tot(&log.active), tot(&log.open));
        let stamps: Vec<f64> = log.active.iter().chain(&log.open).map(|i| i.start).collect();
        out.sessions.push(SessionTime {
            id: s.id.clone(),
            name: s.name.clone(),
            agent: s.agent.clone(),
            state: s.state,
            items: s.items.clone(),
            active,
            open,
            source: if has_active || open > 0 { source } else { String::new() },
            from: stamps.iter().copied().reduce(f64::min),
            to: stamps.iter().copied().reduce(f64::max),
        });
    }
    renames.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let ren: BTreeMap<String, String> = renames.into_iter().flat_map(|(_, m)| m).collect();
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();
    for (items, start, secs, sid, active) in pending {
        if items.is_empty() || secs <= 0.0 {
            continue;
        }
        let share = secs / items.len() as f64;
        let d = day(start);
        for w in items {
            let e = acc.entry(resolve(&w, &ren)).or_default();
            let target = if active { &mut e.0 } else { &mut e.1 };
            *target.entry(d.clone()).or_default() += share;
            if active {
                *e.2.entry(sid.clone()).or_default() += share;
            }
        }
    }
    let round = |m: BTreeMap<String, f64>| m.into_iter().map(|(k, v)| (k, v.round() as u64)).collect();
    out.by_item = acc
        .into_iter()
        .map(|(k, (days, open, sess))| {
            (k, ItemTime { days: round(days), open: round(open), sessions: round(sess) })
        })
        .collect();
    cache.save(&sessions.iter().map(|(s, _)| s.id.as_str()).collect());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(ts: f64, ev: &str, items: &[&str]) -> Json {
        json!({"ts": ts, "ev": ev, "items": items})
    }

    #[test]
    fn active_time_from_the_log() {
        let t0 = 1_800_000_000.0;
        let events = vec![
            ev(t0, "launch", &["a"]),
            ev(t0 + 10.0, "UserPromptSubmit", &["a"]),
            ev(t0 + 70.0, "Stop", &["a"]),
            ev(t0 + 100.0, "UserPromptSubmit", &["a", "b"]),
            ev(t0 + 130.0, "Notification", &["a", "b"]),
            // Un turno sin cierre antes de otro launch: como mucho OPEN_CAP.
            ev(t0 + 200.0, "resume", &["a"]),
            ev(t0 + 200.0 + 3.0 * 3600.0, "launch", &["a"]),
            json!({"ts": t0 + 1.0, "ev": "rename", "map": {"a": "grp/a"}}),
            // Formato de la v1.
            json!({"ts": t0 + 4.0 * 3600.0, "ev": "UserPromptSubmit", "wis": ["c"]}),
            json!({"ts": t0 + 4.0 * 3600.0 + 5.0, "ev": "exit", "wis": ["c"]}),
        ];
        let log = log_intervals(&events, false, t0 + 99999.0);
        let act: Vec<(f64, Vec<String>)> = log.active.iter().map(|i| (i.secs, i.items.clone())).collect();
        assert_eq!(
            act,
            vec![
                (60.0, vec!["a".into()]),
                (30.0, vec!["a".into(), "b".into()]),
                (OPEN_CAP, vec!["a".into()]),
                (5.0, vec!["c".into()]),
            ]
        );
        assert_eq!(log.renames.len(), 1);
        // Launch a launch (sin exit): hasta el último evento; el segundo cierra con el exit.
        assert_eq!(log.open.len(), 2);
        assert_eq!(log.open[0].secs, 200.0);
        // Sigue corriendo: lo abierto cuenta hasta ahora.
        let live = log_intervals(
            &[ev(t0, "launch", &["a"]), ev(t0 + 5.0, "UserPromptSubmit", &["a"])],
            true,
            t0 + 65.0,
        );
        assert_eq!(live.active[0].secs, 60.0);
        assert_eq!(live.open[0].secs, 65.0);
    }

    #[test]
    fn transcript_turns_cap_idle_gaps() {
        let dir = std::env::temp_dir().join(format!("wb-tl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.jsonl");
        let line = |ts: &str, kind: &str, content: Json| {
            json!({"type": kind, "timestamp": ts, "message": {"content": content}}).to_string()
        };
        let text = [
            line("2026-11-19T10:00:00Z", "user", json!("hola")),
            line("2026-11-19T10:01:00Z", "assistant", json!([{"type": "text", "text": "x"}])),
            line("2026-11-19T10:02:00Z", "user", json!([{"type": "tool_result", "content": "ok"}])),
            line("2026-11-19T10:40:00Z", "assistant", json!([])),
            line("2026-11-19T11:00:00Z", "user", json!([{"type": "text", "text": "otra"}])),
            line("2026-11-19T11:00:30Z", "assistant", json!([])),
        ]
        .join("\n");
        std::fs::write(&p, text).unwrap();
        let turns = transcript_turns(&p);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].1, 60.0 + 60.0 + IDLE_CAP);
        assert_eq!(turns[1].1, 30.0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn report_splits_time_between_items_and_follows_renames() {
        let dir = std::env::temp_dir().join(format!("wb-tlr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sessions")).unwrap();
        let t0 = 1_800_000_000.0;
        let rows = [
            ev(t0, "launch", &["a", "b"]),
            ev(t0 + 10.0, "UserPromptSubmit", &["a", "b"]),
            ev(t0 + 110.0, "Stop", &["a", "b"]),
            json!({"ts": t0 + 200.0, "ev": "rename", "map": {"b": "grp-x/b"}}),
            ev(t0 + 300.0, "exit", &["a", "grp-x/b"]),
        ];
        let text: Vec<String> = rows.iter().map(Json::to_string).collect();
        std::fs::write(dir.join("sessions/s1.activity.jsonl"), text.join("\n")).unwrap();
        let s = Session {
            id: "s1".into(),
            name: "uno".into(),
            agent: "claude".into(),
            items: vec!["a".into(), "grp-x/b".into()],
            ..Default::default()
        };
        let r = report(&[(s, false)], &dir, None, t0 + 1000.0);
        assert_eq!(r.by_item["a"].sessions["s1"], 50);
        assert_eq!(r.by_item["grp-x/b"].days.values().sum::<u64>(), 50);
        assert!(!r.by_item.contains_key("b"));
        assert_eq!(r.sessions[0].active, 100);
        assert_eq!(r.sessions[0].open, 300);
        assert_eq!(r.sessions[0].source, "log");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
