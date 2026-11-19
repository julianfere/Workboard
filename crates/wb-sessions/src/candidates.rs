//! Candidatos a tool: scripts que escribieron las sesiones, agrupados por nombre de archivo. Cuentan los
//! que se corrieron `MIN_RUNS` veces o más o que se escribieron en dos sesiones o más. Se descartan los
//! que están dentro de un repo git (código de las apps) y los que ya son tools.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use wb_mcp::tools::{self, Tool};

use crate::stats::SessionStats;

pub const MIN_RUNS: u64 = 3;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// Nombre del archivo.
    pub name: String,
    /// La copia más reciente que todavía existe.
    pub path: String,
    /// Todas las rutas donde se escribió (la más reciente primero).
    pub paths: Vec<String>,
    pub runs: u64,
    pub sessions: Vec<SessionRef>,
    pub items: Vec<String>,
    pub last: Option<f64>,
    pub bytes: u64,
    /// Nombre sugerido para la tool.
    pub suggested: String,
}

fn key(p: &str) -> String {
    if cfg!(windows) { p.replace('/', "\\").to_lowercase() } else { p.to_string() }
}

fn in_git_repo(path: &Path) -> bool {
    path.ancestors().skip(1).any(|p| p.join(".git").exists())
}

#[derive(Default)]
struct Group {
    name: String,
    paths: BTreeMap<String, (String, f64)>,
    sessions: BTreeMap<String, String>,
    runs: u64,
    items: BTreeSet<String>,
    last: f64,
}

pub fn candidates(rows: &[SessionStats], promoted: &[Tool]) -> Vec<Candidate> {
    let origins: BTreeSet<String> =
        promoted.iter().filter_map(|t| t.meta.origin["ruta"].as_str()).map(key).collect();
    let files: BTreeSet<String> = promoted.iter().map(|t| t.meta.file.to_lowercase()).collect();
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for s in rows {
        for w in &s.scripts.written {
            let name = w.path.replace('\\', "/").rsplit('/').next().unwrap_or("").to_string();
            let g = groups.entry(name.to_lowercase()).or_default();
            g.name = name;
            g.paths.insert(key(&w.path), (w.path.clone(), w.ts));
            g.sessions.insert(s.id.clone(), s.name.clone());
            g.items.extend(s.items.iter().cloned());
            g.last = g.last.max(w.ts);
        }
    }
    // Las corridas cuentan en todas las sesiones (también en las que no lo escribieron).
    for s in rows {
        for (k, n) in &s.scripts.runs {
            if let Some(g) = groups.get_mut(k) {
                g.runs += n;
            }
        }
    }
    let mut out: Vec<Candidate> = vec![];
    for (k, g) in groups {
        let mut paths: Vec<(String, f64)> = g.paths.into_values().collect();
        paths.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let Some(path) = paths.iter().map(|p| p.0.clone()).find(|p| Path::new(p).is_file()) else { continue };
        if in_git_repo(Path::new(&path)) || origins.contains(&key(&path)) || files.contains(&k) {
            continue;
        }
        if g.runs < MIN_RUNS && g.sessions.len() < 2 {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        out.push(Candidate {
            suggested: tools::slug(&g.name),
            name: g.name,
            path,
            paths: paths.into_iter().map(|p| p.0).collect(),
            runs: g.runs,
            sessions: g.sessions.into_iter().map(|(id, name)| SessionRef { id, name }).collect(),
            items: g.items.into_iter().collect(),
            last: (g.last > 0.0).then_some(g.last),
            bytes: meta.len(),
        });
    }
    out.sort_by_key(|a| std::cmp::Reverse((a.sessions.len(), a.runs)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{Scripts, Written};

    fn row(id: &str, written: &[&str], runs: &[(&str, u64)]) -> SessionStats {
        SessionStats {
            id: id.into(),
            name: format!("ses {id}"),
            agent: "claude".into(),
            mode: "default".into(),
            plan: false,
            model: None,
            state: crate::registry::State::Paused,
            items: vec![format!("wi-{id}")],
            fork_of: None,
            from: None,
            to: None,
            models: BTreeMap::new(),
            tokens: None,
            prompts: 0,
            interrupts: 0,
            asks: 0,
            tools: BTreeMap::new(),
            scripts: Scripts {
                written: written.iter().map(|p| Written { path: (*p).into(), ts: 1.0, chars: 1 }).collect(),
                runs: runs.iter().map(|(k, n)| ((*k).into(), *n)).collect(),
            },
            outcome: BTreeMap::new(),
            transcript: None,
        }
    }

    #[test]
    fn picks_scripts_run_often_or_written_twice() {
        let dir = std::env::temp_dir().join(format!("wb-cand-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("work")).unwrap();
        std::fs::create_dir_all(dir.join("repo/.git")).unwrap();
        let p = |n: &str| dir.join(n).display().to_string();
        for f in ["work/often.py", "work/twice.ps1", "work/once.py", "repo/code.py"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let rows = vec![
            row(
                "a",
                &[&p("work/often.py"), &p("work/twice.ps1"), &p("work/once.py"), &p("repo/code.py")],
                &[("often.py", 2), ("code.py", 9)],
            ),
            row("b", &[&p("work/twice.ps1")], &[("often.py", 1)]),
        ];
        let c = candidates(&rows, &[]);
        let names: Vec<&str> = c.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, vec!["twice.ps1", "often.py"]);
        assert_eq!(c[1].runs, 3);
        assert_eq!(c[0].sessions.len(), 2);
        assert_eq!(c[0].suggested, "twice");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
