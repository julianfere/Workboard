//! Reflexionar: paquete de evidencia en markdown sobre el trabajo de unos items (qué pasó con cada uno,
//! sus sesiones con sus métricas y dónde leer el detalle) para una sesión en modo plan que propone
//! mejoras y las deja en una nota. El agente lee lo que necesite: acá no va el transcript entero.

use std::path::{Path, PathBuf};

use chrono::TimeZone;
use wb_core::Workspace;
use wb_core::frontmatter::Value;
use wb_core::workspace::README;

use crate::stats::{SessionStats, read_state_log, state_history};

fn fmt_ts(ts: Option<f64>) -> String {
    ts.and_then(|t| chrono::Local.timestamp_opt(t.floor() as i64, 0).earliest())
        .map_or_else(|| "—".into(), |d| d.format("%Y-%m-%d %H:%M").to_string())
}

fn or_dash(s: &str) -> &str {
    if s.is_empty() { "—" } else { s }
}

fn or_empty(s: &str) -> &str {
    if s.is_empty() { "∅" } else { s }
}

/// ¿La sesión trabajó en alguno de esos items (o en su grupo, o en uno de sus miembros)?
pub fn touches(session_items: &[String], names: &[String]) -> bool {
    session_items.iter().any(|w| {
        names.iter().any(|n| w == n || w.starts_with(&format!("{n}/")) || n.starts_with(&format!("{w}/")))
    })
}

/// Lugares con instrucciones vigentes para los agentes (para no proponer lo que ya está).
pub fn instruction_paths(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(project) = root.parent() {
        out.push(project.join("CLAUDE.md"));
        out.push(project.join("AGENTS.md"));
        out.push(project.join(".claude").join("skills"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let claude = PathBuf::from(home).join(".claude");
        out.extend(["CLAUDE.md", "skills", "commands"].map(|n| claude.join(n)));
    }
    out.into_iter().filter(|p| p.exists()).collect()
}

pub fn evidence_text(
    ws: &Workspace,
    names: &[String],
    rows: &[SessionStats],
    config_dir: &Path,
    extra: &[PathBuf],
    lang: &str,
) -> String {
    let t = |k: &str, p: &[(&str, &str)]| wb_i18n::t(lang, k, p);
    let wf = ws.workflow();
    let sess_dir = config_dir.join("sessions");
    let slog = read_state_log(config_dir, ws.root());
    let now = fmt_ts(Some(chrono::Local::now().timestamp() as f64));
    let root = ws.root().display().to_string();
    let mut out = vec![
        t("agent.reflect.title", &[]),
        String::new(),
        t("agent.reflect.generated", &[("date", &now), ("root", &root)]),
        String::new(),
        t("agent.reflect.items", &[]),
        String::new(),
    ];
    for name in names {
        let Ok(folder) = ws.folder_of(name) else { continue };
        let Ok(it) = ws.build_item(&folder) else { continue };
        let ids = if it.ids.is_empty() { String::new() } else { format!(" (#{})", it.ids.join(", #")) };
        out.push(format!("### {name}{ids}"));
        let status = it.meta.get(&wf.fields.status).and_then(Value::as_text).unwrap_or("");
        out.push(t(
            "agent.reflect.itemLine",
            &[("title", or_dash(&it.title)), ("status", or_dash(status)), ("type", or_dash(&it.item_type))],
        ));
        out.push(t("agent.reflect.readme", &[("path", &folder.join(README).display().to_string())]));
        let hist = state_history(ws, name, &slog);
        if !hist.is_empty() {
            let list: Vec<String> = hist
                .iter()
                .map(|h| format!("{} {} → {}", fmt_ts(Some(h.ts)), or_empty(&h.from), or_empty(&h.to)))
                .collect();
            out.push(t("agent.reflect.history", &[("list", &list.join(" · "))]));
        }
        let done = it.tasks.iter().filter(|x| x.done).count().to_string();
        out.push(t("agent.reflect.tasks", &[("done", &done), ("total", &it.tasks.len().to_string())]));
        out.push(String::new());
    }
    out.push(t("agent.reflect.sessions", &[]));
    out.push(String::new());
    let mut mine: Vec<&SessionStats> = rows.iter().filter(|r| touches(&r.items, names)).collect();
    mine.sort_by(|a, b| {
        a.from.unwrap_or(0.0).partial_cmp(&b.from.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal)
    });
    for r in mine {
        out.push(format!("### {}", if r.name.is_empty() { &r.id } else { &r.name }));
        let model = r
            .models
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(m, _)| m.as_str())
            .or(r.model.as_deref())
            .unwrap_or("—");
        let mut line = t(
            "agent.reflect.sessionLine",
            &[("id", &r.id), ("agent", &r.agent), ("model", model), ("mode", &r.mode)],
        );
        if r.plan {
            line.push_str(&t("agent.reflect.planSession", &[]));
        }
        if let Some(f) = &r.fork_of {
            line.push_str(&t("agent.reflect.forkOf", &[("id", f)]));
        }
        out.push(line);
        out.push(t(
            "agent.reflect.window",
            &[("from", &fmt_ts(r.from)), ("to", &fmt_ts(r.to)), ("items", &r.items.join(", "))],
        ));
        out.push(t(
            "agent.reflect.usage",
            &[
                ("prompts", &r.prompts.to_string()),
                ("asks", &r.asks.to_string()),
                ("interrupts", &r.interrupts.to_string()),
            ],
        ));
        if let Some(tk) = &r.tokens {
            out.push(t(
                "agent.reflect.tokens",
                &[
                    ("output", &tk.output.to_string()),
                    ("input", &tk.input.to_string()),
                    ("cacheRead", &tk.cache_read.to_string()),
                    ("cacheWrite", &tk.cache_write.to_string()),
                ],
            ));
        }
        let mut tools: Vec<(&String, &u64)> = r.tools.iter().collect();
        tools.sort_by(|a, b| b.1.cmp(a.1));
        let top: Vec<String> = tools.iter().take(8).map(|(k, v)| format!("{k} {v}")).collect();
        let top = if top.is_empty() { "—".to_string() } else { top.join(", ") };
        out.push(t("agent.reflect.tools", &[("list", &top)]));
        if !r.scripts.written.is_empty() {
            let list: Vec<String> = r.scripts.written.iter().map(|w| format!("`{}`", w.path)).collect();
            out.push(t("agent.reflect.scripts", &[("list", &list.join(", "))]));
        }
        for (w, res) in &r.outcome {
            if res.transitions.is_empty() {
                continue;
            }
            let steps: Vec<String> = res
                .transitions
                .iter()
                .map(|x| format!("{} → {}", or_empty(&x.from), or_empty(&x.to)))
                .collect();
            let mut l = format!("- {w}: {}", steps.join(" · "));
            if res.setbacks > 0 {
                l.push_str(&t("agent.reflect.rework", &[("n", &res.setbacks.to_string())]));
            }
            out.push(l);
        }
        if let Some(p) = &r.transcript {
            out.push(t("agent.reflect.transcript", &[("path", p)]));
        }
        for (suffix, key) in
            [(".activity.jsonl", "agent.reflect.activity"), (".events.jsonl", "agent.reflect.events")]
        {
            let p = sess_dir.join(format!("{}{suffix}", r.id));
            if p.exists() {
                out.push(t(key, &[("path", &p.display().to_string())]));
            }
        }
        out.push(String::new());
    }
    if !extra.is_empty() {
        out.push(t("agent.reflect.instructions", &[]));
        out.push(String::new());
        out.extend(extra.iter().map(|p| format!("- `{}`", p.display())));
        out.push(String::new());
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touches_groups_and_members() {
        let n = vec!["grp-a".to_string()];
        assert!(touches(&["grp-a/wi1".into()], &n));
        assert!(touches(&["grp-a".into()], &["grp-a/wi1".into()]));
        assert!(!touches(&["grp-ab".into()], &n));
    }
}
