//! `workboard migrate-v1`: trae la configuración de la v1 (la app Python, `%APPDATA%\Workboard`) a la
//! carpeta de config de la v2. La tabla de qué se migra, qué se transforma y qué se ignora está en
//! `docs/migrate-v1.md`.
//!
//! - Nunca escribe en la carpeta de la v1 (solo la lee).
//! - Idempotente: lo que ya está igual en la v2 no se toca; lo que la v2 tiene distinto se conserva salvo
//!   `--force`. Los registros (marcas de ejecutados, `state_log.jsonl`, actividad de sesiones, backups,
//!   notas) se suman: solo se agrega lo que falta.
//! - `--dry-run` arma el mismo plan sin escribir nada (ni archivos ni el almacén de credenciales).
//! - Los PATs (DPAPI en la v1) van al almacén de credenciales con las claves de los trackers de la v2. Nunca
//!   aparecen en el reporte ni en los errores.
//!
//! El reporte son códigos (`migrate.*`) con parámetros; [`render`] los traduce.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{Value as Json, json};
use wb_core::Workflow;
use wb_core::records::RunLog;
use wb_platform::secrets::SecretStore;
use wb_sessions::{Session, State};
use wb_tracker::{Connection, TrackerState, azure};

use crate::settings::{self, MAX_RECENTS, Settings, Theme};
use crate::tracker::{self, FieldRef, ProjectCfg, TrackerFile};
use crate::{ApiError, ApiResult};

/// Preset que equivale al flujo de la v1.
pub const DEFAULT_PRESET: &str = "classic-es";
/// La v1 era solo en castellano.
const V1_LANG: &str = "es";

/// Descifra un PAT de la v1 (en la app, `wb_platform::dpapi::unprotect`; los tests pasan otro).
pub type Unprotect<'a> = &'a dyn Fn(&str) -> io::Result<String>;

#[derive(Debug, Clone)]
pub struct Options {
    /// Carpeta de config de la v1.
    pub from: PathBuf,
    /// Carpeta de config de la v2.
    pub to: PathBuf,
    /// Preset de flujo (el incluido `classic-es` o uno de un plugin habilitado en la v2).
    pub preset: String,
    pub dry_run: bool,
    pub force: bool,
}

/// Qué pasó con cada cosa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Escrito en la v2.
    Done,
    /// Se escribiría (`--dry-run`).
    Planned,
    /// La v2 ya lo tiene igual.
    Same,
    /// La v2 tiene otro valor: se conserva (`--force` lo reemplaza).
    Kept,
    Failed,
}

impl Outcome {
    pub fn key(self) -> &'static str {
        match self {
            Outcome::Done => "migrate.status.done",
            Outcome::Planned => "migrate.status.planned",
            Outcome::Same => "migrate.status.same",
            Outcome::Kept => "migrate.status.kept",
            Outcome::Failed => "migrate.status.failed",
        }
    }
}

/// Una línea del reporte: `item` y `note` son claves de `locales/*.json` con los mismos `params`.
#[derive(Debug, Clone)]
pub struct Entry {
    pub outcome: Outcome,
    pub item: &'static str,
    pub params: Vec<(&'static str, String)>,
    pub note: Option<&'static str>,
}

impl Entry {
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Report {
    pub from: PathBuf,
    pub to: PathBuf,
    pub dry_run: bool,
    /// Idioma de la v2 después de migrar (para el reporte).
    pub lang: String,
    pub entries: Vec<Entry>,
    /// Lo que hay en la v1 y no se migra (claves `migrate.ignored.*`).
    pub ignored: Vec<&'static str>,
}

impl Report {
    pub fn count(&self, outcome: Outcome) -> usize {
        self.entries.iter().filter(|e| e.outcome == outcome).count()
    }

    pub fn find(&self, item: &str) -> Vec<&Entry> {
        self.entries.iter().filter(|e| e.item == item).collect()
    }

    pub fn failed(&self) -> bool {
        self.count(Outcome::Failed) > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    Write,
    Same,
    Kept,
}

fn plan_value<T: PartialEq>(current: Option<&T>, new: &T, force: bool) -> Plan {
    match current {
        None => Plan::Write,
        Some(c) if c == new => Plan::Same,
        Some(_) if force => Plan::Write,
        Some(_) => Plan::Kept,
    }
}

/// Plan de un registro que se suma: `added` cosas nuevas, `kept` distintas que se conservan.
fn plan_counts(added: usize, kept: usize) -> Plan {
    if added > 0 {
        Plan::Write
    } else if kept > 0 {
        Plan::Kept
    } else {
        Plan::Same
    }
}

struct Run<'a> {
    o: &'a Options,
    secrets: &'a dyn SecretStore,
    unprotect: Unprotect<'a>,
    wf: Workflow,
    entries: Vec<Entry>,
}

impl Run<'_> {
    /// Anota una línea; devuelve si hay que escribir (plan de escritura y no es `--dry-run`).
    fn record(&mut self, plan: Plan, item: &'static str, params: Vec<(&'static str, String)>) -> bool {
        let outcome = match plan {
            Plan::Write if self.o.dry_run => Outcome::Planned,
            Plan::Write => Outcome::Done,
            Plan::Same => Outcome::Same,
            Plan::Kept => Outcome::Kept,
        };
        self.entries.push(Entry { outcome, item, params, note: None });
        outcome == Outcome::Done
    }

    fn fail(
        &mut self,
        item: &'static str,
        mut params: Vec<(&'static str, String)>,
        note: &'static str,
        why: String,
    ) {
        params.push(("error", why));
        self.entries.push(Entry { outcome: Outcome::Failed, item, params, note: Some(note) });
    }

    /// La última línea anotada falló al escribir.
    fn failed_last(&mut self, e: impl std::fmt::Display) {
        if let Some(last) = self.entries.last_mut() {
            last.outcome = Outcome::Failed;
            last.note = Some("migrate.note.error");
            last.params.push(("error", e.to_string()));
        }
    }

    fn note_last(&mut self, note: &'static str) {
        if let Some(last) = self.entries.last_mut()
            && last.note.is_none()
        {
            last.note = Some(note);
        }
    }

    /// Escribe (si corresponde) con `write`; un error marca la línea como fallida.
    fn apply(
        &mut self,
        plan: Plan,
        item: &'static str,
        params: Vec<(&'static str, String)>,
        write: impl FnOnce() -> io::Result<()>,
    ) {
        if self.record(plan, item, params)
            && let Err(e) = write()
        {
            self.failed_last(e);
        }
    }
}

// ---------- utilidades ----------

fn read_json(path: &Path) -> Json {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or(Json::Null)
}

fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".migrate.tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> io::Result<()> {
    write_atomic(path, serde_json::to_string_pretty(value).map_err(io::Error::other)?.as_bytes())
}

fn str_of(v: &Json) -> Option<String> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn str_list(v: &Json) -> Vec<String> {
    v.as_array().map_or(vec![], |l| l.iter().filter_map(str_of).collect())
}

/// Ruta como la guarda la v2: canónica si existe (sin `\\?\`), si no tal cual.
fn canon(path: &str) -> String {
    dunce::canonicalize(path).map_or_else(|_| path.to_string(), |p| p.display().to_string())
}

/// `os.path.normcase` de la v1 (en Windows, minúsculas y `\`).
fn normcase(path: &str) -> String {
    if cfg!(windows) { path.replace('/', "\\").to_lowercase() } else { path.to_string() }
}

/// Archivos (recursivo) bajo `dir`, como rutas relativas. Una carpeta que no existe no tiene archivos.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(base, &p, out),
                Ok(t) if t.is_file() => {
                    if let Ok(rel) = p.strip_prefix(base) {
                        out.push(rel.to_path_buf());
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = vec![];
    walk(dir, dir, &mut out);
    out
}

/// Copia un árbol sin pisar: cuenta lo nuevo y lo distinto (que se pisa solo con `force`).
struct TreeCopy {
    copies: Vec<(PathBuf, PathBuf)>,
    added: usize,
    kept: usize,
}

fn plan_tree(src: &Path, dst: &Path, force: bool) -> TreeCopy {
    let mut out = TreeCopy { copies: vec![], added: 0, kept: 0 };
    for rel in files_under(src) {
        let (a, b) = (src.join(&rel), dst.join(&rel));
        if b.exists() {
            if std::fs::read(&a).ok() == std::fs::read(&b).ok() {
                continue;
            }
            if !force {
                out.kept += 1;
                continue;
            }
        }
        out.added += 1;
        out.copies.push((a, b));
    }
    out
}

fn copy_files(copies: &[(PathBuf, PathBuf)]) -> io::Result<()> {
    for (a, b) in copies {
        write_atomic(b, &std::fs::read(a)?)?;
    }
    Ok(())
}

/// Une dos registros `.jsonl`: agrega las filas de `src` que faltan en `dst` y ordena por `ts` (estable).
/// Devuelve el texto nuevo y cuántas filas se agregaron.
fn merge_jsonl(src: &Path, dst: &Path) -> io::Result<(String, usize)> {
    let read = |p: &Path| -> io::Result<Vec<String>> {
        match std::fs::read_to_string(p) {
            Ok(t) => Ok(t
                .trim_start_matches('\u{feff}')
                .lines()
                .map(str::trim_end)
                .filter(|l| !l.trim().is_empty())
                .map(String::from)
                .collect()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e),
        }
    };
    let (new, mut lines) = (read(src)?, read(dst)?);
    let seen: BTreeSet<String> = lines.iter().cloned().collect();
    let mut added = 0;
    let mut fresh = BTreeSet::new();
    for l in new {
        if !seen.contains(&l) && fresh.insert(l.clone()) {
            lines.push(l);
            added += 1;
        }
    }
    let ts = |l: &String| serde_json::from_str::<Json>(l).ok().and_then(|v| v["ts"].as_f64()).unwrap_or(0.0);
    lines.sort_by(|a, b| ts(a).total_cmp(&ts(b)));
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    Ok((text, added))
}

// ---------- pasos ----------

/// Raíz de la v1 con la forma en que la escribió el usuario (la v1 guarda varias claves en minúsculas).
fn display_root(v1: &Json, key: &str) -> String {
    let mut known = str_list(&v1["recents"]);
    known.extend(str_of(&v1["root"]));
    known.into_iter().find(|r| normcase(r) == normcase(key)).unwrap_or_else(|| key.to_string())
}

fn migrate_settings(run: &mut Run, v1: &Json) -> Settings {
    let path = settings::path_in(&run.o.to);
    let mut s = settings::load(&path);
    let force = run.o.force;
    let mut changed = false;
    let default = Settings::default();

    // Preset: el valor por defecto de la v2 cuenta como «sin elegir».
    let current = (s.workflow != default.workflow).then_some(&s.workflow);
    let plan = plan_value(current, &run.o.preset, force);
    run.record(plan, "migrate.item.workflow", vec![("preset", run.o.preset.clone())]);
    if plan == Plan::Write {
        s.workflow = run.o.preset.clone();
        changed = true;
    }

    let lang = V1_LANG.to_string();
    let plan = plan_value(s.lang.as_ref(), &lang, force);
    run.record(plan, "migrate.item.lang", vec![("lang", lang.clone())]);
    if plan == Plan::Write {
        s.lang = Some(lang);
        changed = true;
    }

    let theme = match v1["prefs"]["theme"].as_str() {
        Some("light") => Some(Theme::Light),
        Some("dark") => Some(Theme::Dark),
        _ => None,
    };
    if let Some(theme) = theme {
        let current = (s.theme != Theme::System).then_some(&s.theme);
        let plan = plan_value(current, &theme, force);
        let name = if theme == Theme::Dark { "dark" } else { "light" };
        run.record(plan, "migrate.item.theme", vec![("theme", name.into())]);
        if plan == Plan::Write {
            s.theme = theme;
            changed = true;
        }
    }

    if let Some(root) = str_of(&v1["root"]) {
        let root = canon(&root);
        let plan = plan_value(s.root.as_ref(), &root, force);
        run.record(plan, "migrate.item.root", vec![("path", root.clone())]);
        if plan == Plan::Write {
            s.root = Some(root);
            changed = true;
        }
    }

    let mut added = 0;
    for r in str_list(&v1["recents"]).iter().map(|r| canon(r)) {
        if s.recents.len() < MAX_RECENTS && !s.recents.iter().any(|x| x.eq_ignore_ascii_case(&r)) {
            s.recents.push(r);
            added += 1;
        }
    }
    if !str_list(&v1["recents"]).is_empty() {
        run.record(plan_counts(added, 0), "migrate.item.recents", vec![("count", added.to_string())]);
        changed |= added > 0;
    }
    // La raíz abierta va primera entre los recientes, como en la app.
    if let Some(root) = s.root.clone()
        && changed
        && s.recents.first().is_none_or(|f| !f.eq_ignore_ascii_case(&root))
    {
        s.open_root(&root);
    }

    if let Some(folder) = str_of(&v1["notesFolder"]) {
        let plan = plan_value(s.notes_folder.as_ref(), &folder, force);
        run.record(plan, "migrate.item.notesFolder", vec![("path", folder.clone())]);
        if plan == Plan::Write {
            s.notes_folder = Some(folder);
            changed = true;
        }
    }

    let agents = &v1["agents"];
    if let Some(agent) = str_of(&agents["default"]).filter(|a| wb_sessions::agents::agent(a).is_some()) {
        let current = (!s.agents.default.is_empty()).then_some(&s.agents.default);
        let plan = plan_value(current, &agent, force);
        run.record(plan, "migrate.item.agentDefault", vec![("agent", agent.clone())]);
        if plan == Plan::Write {
            s.agents.default = agent;
            changed = true;
        }
    }
    let c = &agents["custom"];
    let custom = wb_sessions::CustomAgent {
        label: str_of(&c["label"]).unwrap_or_default(),
        command: str_of(&c["command"]).unwrap_or_default(),
        args: str_of(&c["args"]).unwrap_or_default(),
        resume_args: str_of(&c["resumeArgs"]).unwrap_or_default(),
    };
    if custom.is_set() {
        let current = (s.agents.custom != wb_sessions::CustomAgent::default()).then_some(&s.agents.custom);
        let plan = plan_value(current, &custom, force);
        run.record(plan, "migrate.item.agentCustom", vec![("command", custom.command.clone())]);
        if plan == Plan::Write {
            s.agents.custom = custom;
            changed = true;
        }
    }

    // Repos del panel Git por raíz. La v1 guardaba los ambientes en mayúsculas (`DEV`).
    if let Some(git) = v1["git"].as_object() {
        for (key, list) in git {
            let root = canon(&display_root(v1, key));
            let repos: Vec<wb_git::RepoConfig> = list
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| {
                    let path = str_of(&r["path"])?;
                    let envs: IndexMap<String, String> = run
                        .wf
                        .environments
                        .iter()
                        .filter_map(|e| {
                            let (_, v) =
                                r["envs"].as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(&e.id))?;
                            Some((e.id.clone(), str_of(v)?))
                        })
                        .collect();
                    let name = str_of(&r["name"]).unwrap_or_else(|| {
                        Path::new(&path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    });
                    Some(wb_git::RepoConfig { name, path: canon(&path), envs, compare: true })
                })
                .collect();
            let plan = plan_value(s.git.repos.get(&root), &repos, force);
            run.record(
                plan,
                "migrate.item.gitRepos",
                vec![("root", root.clone()), ("count", repos.len().to_string())],
            );
            if plan == Plan::Write {
                s.git.repos.insert(root, repos);
                changed = true;
            }
        }
    }

    if changed && !run.o.dry_run {
        let first = run.entries.len();
        if let Err(e) = settings::save(&path, &s) {
            // Todo lo de `settings.json` falló junto.
            for entry in &mut run.entries[..first] {
                if entry.outcome == Outcome::Done {
                    entry.outcome = Outcome::Failed;
                    entry.note = Some("migrate.note.error");
                    entry.params.push(("error", e.to_string()));
                }
            }
        }
    }
    s
}

fn tracker_state(v: &Json) -> Option<TrackerState> {
    let category = str_of(&v["category"]).unwrap_or_default();
    Some(TrackerState {
        name: str_of(&v["name"])?,
        color: str_of(&v["color"]).unwrap_or_default(),
        suggested: azure::category_of(&category),
        category,
        types: str_list(&v["types"]),
    })
}

fn field_refs(v: &Json) -> Option<Vec<FieldRef>> {
    Some(
        v.as_array()?
            .iter()
            .filter_map(|f| {
                Some(FieldRef { reference: str_of(&f["ref"])?, name: str_of(&f["name"]).unwrap_or_default() })
            })
            .collect(),
    )
}

/// Proyecto de la v1 (`ado.procs["org|proyecto"]`) en el formato de la v2; cuenta los valores del mapeo
/// que el preset no tiene.
fn project_cfg(wf: &Workflow, p: &Json) -> (ProjectCfg, usize) {
    let valid: BTreeSet<&str> = wf.states.iter().map(|s| s.id.as_str()).collect();
    let mut dropped = 0;
    let map: BTreeMap<String, String> = p["map"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| {
            let v = str_of(v)?;
            if valid.contains(v.as_str()) {
                Some((k.clone(), v))
            } else {
                dropped += 1;
                None
            }
        })
        .collect();
    let cfg = ProjectCfg {
        states: p["states"].as_array().into_iter().flatten().filter_map(tracker_state).collect(),
        fetched_at: p["fetchedAt"].as_f64(),
        map,
        map_saved: p["mapSaved"].as_bool().unwrap_or(false),
        date_fields: field_refs(&p["dateFields"]),
        num_fields: field_refs(&p["numFields"]),
        has_priority: p["hasPriority"].as_bool().unwrap_or(false),
        has_effort: p["hasEffort"].as_bool().unwrap_or(false),
        // "" = el usuario eligió «ninguno»: se conserva.
        due_field: p["pactadaField"].as_str().map(String::from),
        hours_field: p["horasField"].as_str().map(String::from),
    };
    (cfg, dropped)
}

fn same_json<T: serde::Serialize>(a: &T, b: &T) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

fn migrate_trackers(run: &mut Run, v1: &Json) {
    let ado = &v1["ado"];
    let path = run.o.to.join(tracker::TRACKERS_FILE);
    let mut file: TrackerFile = serde_json::from_value(read_json(&path)).unwrap_or_default();
    let mut changed = false;
    let first = run.entries.len();

    for (key, c) in ado["conn"].as_object().into_iter().flatten() {
        let root = display_root(v1, key);
        let project = str_of(&c["project"]).unwrap_or_default();
        let params = vec![("root", root.clone()), ("project", project.clone())];
        let conn = match Connection::normalized(azure::KIND, c["org"].as_str().unwrap_or(""), &project) {
            Ok(conn) => conn,
            Err(e) => {
                run.fail("migrate.item.connection", params, "migrate.note.error", e.code.to_string());
                continue;
            }
        };
        let root_key = RunLog::root_key(Path::new(&root));
        let plan = plan_value(file.connections.get(&root_key), &conn, run.o.force);
        run.record(plan, "migrate.item.connection", params);
        if plan == Plan::Write {
            file.connections.insert(root_key, conn);
            changed = true;
        }
    }

    for (key, p) in ado["procs"].as_object().into_iter().flatten() {
        let (org, project) = key.split_once('|').unwrap_or((key, ""));
        let params = vec![("project", project.to_string())];
        let conn = match Connection::normalized(azure::KIND, org, project) {
            Ok(conn) => conn,
            Err(e) => {
                run.fail("migrate.item.project", params, "migrate.note.error", e.code.to_string());
                continue;
            }
        };
        let (cfg, dropped) = project_cfg(&run.wf, p);
        let pkey = tracker::project_key(&conn);
        let plan = match file.projects.get(&pkey) {
            None => Plan::Write,
            Some(cur) if same_json(cur, &cfg) => Plan::Same,
            Some(_) if run.o.force => Plan::Write,
            Some(_) => Plan::Kept,
        };
        let mut params = params;
        params.push(("states", cfg.map.len().to_string()));
        params.push(("dropped", dropped.to_string()));
        run.record(plan, "migrate.item.project", params);
        if dropped > 0 {
            run.note_last("migrate.note.mapDropped");
        }
        if plan == Plan::Write {
            file.projects.insert(pkey, cfg);
            changed = true;
        }
    }

    if changed
        && !run.o.dry_run
        && let Err(e) = write_json(&path, &file)
    {
        for entry in &mut run.entries[first..] {
            if entry.outcome == Outcome::Done {
                entry.outcome = Outcome::Failed;
                entry.note = Some("migrate.note.error");
                entry.params.push(("error", e.to_string()));
            }
        }
    }

    // PATs: el de sincronizar (lectura) y el de escritura, por organización.
    for (field, write, item) in
        [("pats", false, "migrate.item.secretRead"), ("wpats", true, "migrate.item.secretWrite")]
    {
        for (org, token) in ado[field].as_object().into_iter().flatten() {
            let params = vec![("org", org.clone())];
            let Some(token) = token.as_str().filter(|t| !t.is_empty()) else { continue };
            let conn = match azure::normalize_org(org) {
                Ok(org) => Connection { kind: azure::KIND.into(), org, project: String::new() },
                Err(e) => {
                    run.fail(item, params, "migrate.note.error", e.code.to_string());
                    continue;
                }
            };
            let secret = match (run.unprotect)(token) {
                Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
                _ => {
                    run.fail(item, params, "migrate.note.decrypt", String::new());
                    continue;
                }
            };
            let key = tracker::secret_key(&conn, write);
            let current = match run.secrets.get(&key) {
                Ok(v) => v,
                Err(_) => {
                    run.fail(item, params, "migrate.note.store", String::new());
                    continue;
                }
            };
            let plan = plan_value(current.as_ref(), &secret, run.o.force);
            if run.record(plan, item, params) && run.secrets.set(&key, &secret).is_err() {
                run.failed_last("");
                run.entries.last_mut().expect("entry").note = Some("migrate.note.store");
            }
        }
    }
}

fn migrate_runs(run: &mut Run) {
    let src = run.o.from.join("runs.json");
    let Some(v1) = read_json(&src).as_object().cloned() else { return };
    let path = run.o.to.join("runs.json");
    let mut data: BTreeMap<String, BTreeMap<String, BTreeMap<String, Json>>> =
        serde_json::from_value(read_json(&path)).unwrap_or_default();
    let (mut added, mut kept) = (0, 0);
    for (root, files) in v1 {
        let key = RunLog::root_key(Path::new(&root));
        for (rel, marks) in files.as_object().into_iter().flatten() {
            for (env, ts) in marks.as_object().into_iter().flatten() {
                let slot = data.entry(key.clone()).or_default().entry(rel.clone()).or_default();
                match slot.get(env) {
                    Some(cur) if cur == ts => {}
                    Some(_) if !run.o.force => kept += 1,
                    _ => {
                        slot.insert(env.clone(), ts.clone());
                        added += 1;
                    }
                }
            }
        }
    }
    let plan = plan_counts(added, kept);
    run.apply(
        plan,
        "migrate.item.runs",
        vec![("count", added.to_string()), ("kept", kept.to_string())],
        || write_json(&path, &data),
    );
}

/// Registros `.jsonl` de la carpeta de config (`state_log.jsonl`, `mcp_events.jsonl`).
fn migrate_jsonl(run: &mut Run, name: &str, item: &'static str) {
    let src = run.o.from.join(name);
    if !src.is_file() {
        return;
    }
    let dst = run.o.to.join(name);
    match merge_jsonl(&src, &dst) {
        Ok((text, added)) => {
            run.apply(plan_counts(added, 0), item, vec![("count", added.to_string())], || {
                write_atomic(&dst, text.as_bytes())
            });
        }
        Err(e) => run.fail(item, vec![("count", "0".into())], "migrate.note.error", e.to_string()),
    }
}

fn migrate_tools(run: &mut Run) {
    let src = run.o.from.join("tools");
    let Ok(rd) = std::fs::read_dir(&src) else { return };
    let mut dirs: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.join("tool.json").is_file()).collect();
    dirs.sort();
    for dir in dirs {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // `_removed`: las quitadas no se traen.
        if name.starts_with('_') {
            continue;
        }
        let meta = read_json(&dir.join("tool.json"));
        let hash_ok = match (str_of(&meta["archivo"]), str_of(&meta["sha256"])) {
            (Some(file), Some(sha)) => {
                wb_sessions::tools::sha256_file(&dir.join(file)).is_ok_and(|h| h.eq_ignore_ascii_case(&sha))
            }
            _ => false,
        };
        let dst = run.o.to.join("tools").join(&name);
        let copy = plan_tree(&dir, &dst, run.o.force);
        let plan = plan_counts(copy.added, copy.kept);
        run.apply(plan, "migrate.item.tool", vec![("name", name)], || copy_files(&copy.copies));
        if !hash_ok {
            run.note_last("migrate.note.toolHash");
        }
    }
}

fn migrate_file(run: &mut Run, name: &str, item: &'static str) {
    let src = run.o.from.join(name);
    let Ok(data) = std::fs::read(&src) else { return };
    let dst = run.o.to.join(name);
    let current = std::fs::read(&dst).ok();
    let plan = plan_value(current.as_ref(), &data, run.o.force);
    run.apply(plan, item, vec![], || write_atomic(&dst, &data));
}

/// Copia sin pisar un árbol de la config de la v1 a otro lugar.
fn migrate_tree(run: &mut Run, src: &Path, dst: &Path, item: &'static str) {
    if !src.is_dir() {
        return;
    }
    let copy = plan_tree(src, dst, run.o.force);
    let params = vec![("count", copy.added.to_string()), ("kept", copy.kept.to_string())];
    run.apply(plan_counts(copy.added, copy.kept), item, params, || copy_files(&copy.copies));
}

/// Sesión del registro de la v1 (claves en castellano) en el formato de la v2.
fn v1_session(v: &Json) -> Option<Session> {
    let s = |k: &str| str_of(&v[k]).unwrap_or_default();
    Some(Session {
        id: str_of(&v["id"])?,
        name: s("nombre"),
        root: s("raiz"),
        cwd: s("cwd"),
        items: str_list(&v["wis"]),
        // Ninguna sesión migrada tiene proceso: se retoman con Reanudar.
        state: if v["estado"] == "cerrada" { State::Closed } else { State::Paused },
        mode: str_of(&v["modo"]).unwrap_or_else(|| "default".into()),
        agent: str_of(&v["agente"]).unwrap_or_else(|| wb_sessions::agents::DEFAULT_AGENT.into()),
        plan: v["plan"].as_bool().unwrap_or(false),
        model: str_of(&v["modelo"]),
        created: s("creada"),
        last_activity: s("ultimaActividad"),
        fork_of: str_of(&v["forkDe"]),
        repos: str_list(&v["repos"]),
        pinned: false,
        keep_open: false,
    })
}

fn migrate_sessions(run: &mut Run) {
    let src = run.o.from.join("sessions");
    let Ok(rd) = std::fs::read_dir(&src) else { return };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    files.sort();
    let dst_dir = run.o.to.join("sessions");
    let (mut log_added, mut log_files, mut log_seen) = (0, 0, 0);
    let mut log_writes: Vec<(PathBuf, String)> = vec![];
    for file in files {
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dst = dst_dir.join(&name);
        if name.ends_with(".activity.jsonl") || name.ends_with(".events.jsonl") {
            log_seen += 1;
            match merge_jsonl(&file, &dst) {
                Ok((text, added)) if added > 0 => {
                    log_added += added;
                    log_files += 1;
                    log_writes.push((dst, text));
                }
                Ok(_) => {}
                Err(e) => run.fail(
                    "migrate.item.sessionLogs",
                    vec![("count", "0".into()), ("files", "0".into())],
                    "migrate.note.error",
                    e.to_string(),
                ),
            }
            continue;
        }
        if !name.ends_with(".json") {
            continue;
        }
        // Registro de una raíz: `{"sessions": [...]}`; mismo nombre de archivo en las dos versiones.
        let v1 = read_json(&file);
        let list: Vec<Session> =
            v1["sessions"].as_array().into_iter().flatten().filter_map(v1_session).collect();
        if list.is_empty() {
            continue;
        }
        let root = list[0].root.clone();
        let mut current: Vec<Session> =
            serde_json::from_value(read_json(&dst)["sessions"].clone()).unwrap_or_default();
        let (mut added, mut kept) = (0, 0);
        for s in list {
            match current.iter_mut().find(|c| c.id == s.id) {
                None => {
                    current.push(s);
                    added += 1;
                }
                Some(c) if *c == s => {}
                Some(c) if run.o.force => {
                    *c = s;
                    added += 1;
                }
                Some(_) => kept += 1,
            }
        }
        let params = vec![("root", root), ("count", added.to_string()), ("kept", kept.to_string())];
        run.apply(plan_counts(added, kept), "migrate.item.sessions", params, || {
            write_json(&dst, &json!({ "sessions": current }))
        });
    }
    if log_seen > 0 {
        let params = vec![("count", log_added.to_string()), ("files", log_files.to_string())];
        run.apply(plan_counts(log_added, 0), "migrate.item.sessionLogs", params, || {
            log_writes.iter().try_for_each(|(p, t)| write_atomic(p, t.as_bytes()))
        });
    }
}

/// Lo que la v1 tiene y no se migra (se regenera, no existe en la v2 o es de la ventana).
fn ignored(from: &Path, v1: &Json) -> Vec<&'static str> {
    let has = |name: &str| from.join(name).exists();
    let mut out = vec![];
    if ["ado_cache.json", "git_cache.json", "time_cache.json", "stats_cache.json"].iter().any(|f| has(f)) {
        out.push("migrate.ignored.caches");
    }
    if has("workboard.log") {
        out.push("migrate.ignored.log");
    }
    if has("webview") {
        out.push("migrate.ignored.webview");
    }
    if has("ctx") || has("claude") {
        out.push("migrate.ignored.sessionFiles");
    }
    if has("reflect") || has("review") {
        out.push("migrate.ignored.packs");
    }
    if has("notas-borradas") {
        out.push("migrate.ignored.deletedNotes");
    }
    if has("tools/_removed") {
        out.push("migrate.ignored.removedTools");
    }
    if v1["aliases"].as_object().is_some_and(|a| !a.is_empty()) {
        out.push("migrate.ignored.aliases");
    }
    if v1["prefs"].as_object().is_some_and(|p| p.keys().any(|k| k != "theme")) {
        out.push("migrate.ignored.prefs");
    }
    if v1["ado"]["rm"].as_object().is_some_and(|r| !r.is_empty()) {
        out.push("migrate.ignored.releaseManual");
    }
    out
}

/// Migra según `o`. Los secretos van a `secrets`; `unprotect` descifra los PATs de la v1.
pub fn run(o: &Options, secrets: &dyn SecretStore, unprotect: Unprotect) -> ApiResult<Report> {
    if !o.from.is_dir() {
        return Err(ApiError::new("migrate.error.noV1", &[("path", &o.from.display().to_string())]));
    }
    if canon(&o.from.display().to_string()).eq_ignore_ascii_case(&canon(&o.to.display().to_string())) {
        return Err(ApiError::new("migrate.error.sameFolder", &[]));
    }
    let wf = wb_plugins::resolve_workflow(&o.to, &o.preset)
        .map_err(|_| ApiError::new("migrate.error.preset", &[("preset", &o.preset)]))?;
    let v1 = match read_json(&o.from.join("config.json")) {
        Json::Object(m) => Json::Object(m),
        _ => json!({}),
    };
    let mut r = Run { o, secrets, unprotect, wf, entries: vec![] };

    let s = migrate_settings(&mut r, &v1);
    migrate_trackers(&mut r, &v1);
    migrate_runs(&mut r);
    migrate_jsonl(&mut r, "state_log.jsonl", "migrate.item.stateLog");
    migrate_jsonl(&mut r, "mcp_events.jsonl", "migrate.item.mcpEvents");
    migrate_tools(&mut r);
    migrate_file(&mut r, "inbox.md", "migrate.item.inbox");
    // Notas: solo las de la carpeta por defecto de la v1 (una carpeta propia queda donde está y se apunta
    // con `notesFolder`).
    if str_of(&v1["notesFolder"]).is_none() {
        let dst = s.notes_folder.as_ref().map_or_else(|| o.to.join("notes"), PathBuf::from);
        migrate_tree(&mut r, &o.from.join("notas"), &dst, "migrate.item.notes");
    }
    migrate_tree(&mut r, &o.from.join("backups"), &o.to.join("backups"), "migrate.item.backups");
    migrate_sessions(&mut r);

    Ok(Report {
        from: o.from.clone(),
        to: o.to.clone(),
        dry_run: o.dry_run,
        lang: s.lang.unwrap_or_else(|| wb_i18n::DEFAULT_LANG.into()),
        entries: r.entries,
        ignored: ignored(&o.from, &v1),
    })
}

/// El reporte como texto, en `lang`.
pub fn render(report: &Report, lang: &str) -> String {
    let t = |key: &str, params: &[(&str, &str)]| wb_i18n::t(lang, key, params);
    let mut out = vec![t(
        "migrate.header",
        &[("from", &report.from.display().to_string()), ("to", &report.to.display().to_string())],
    )];
    if report.dry_run {
        out.push(t("migrate.dryRun", &[]));
    }
    out.push(String::new());
    let labels: Vec<String> =
        [Outcome::Done, Outcome::Planned, Outcome::Same, Outcome::Kept, Outcome::Failed]
            .iter()
            .map(|o| t(o.key(), &[]))
            .collect();
    let width = labels.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    for e in &report.entries {
        let params: Vec<(&str, &str)> = e.params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let label = t(e.outcome.key(), &[]);
        let pad = " ".repeat(width.saturating_sub(label.chars().count()));
        let mut line = format!("  [{label}]{pad} {}", t(e.item, &params));
        if let Some(note) = e.note {
            line.push_str(" — ");
            line.push_str(&t(note, &params));
        }
        out.push(line);
    }
    if report.entries.is_empty() {
        out.push(format!("  {}", t("migrate.nothing", &[])));
    }
    if !report.ignored.is_empty() {
        out.push(String::new());
        out.push(t("migrate.ignoredHeader", &[]));
        out.extend(report.ignored.iter().map(|k| format!("  - {}", t(k, &[]))));
    }
    out.push(String::new());
    let n = |o| report.count(o).to_string();
    let done = if report.dry_run { n(Outcome::Planned) } else { n(Outcome::Done) };
    let summary = if report.dry_run { "migrate.summaryDry" } else { "migrate.summary" };
    out.push(t(
        summary,
        &[
            ("done", &done),
            ("same", &n(Outcome::Same)),
            ("kept", &n(Outcome::Kept)),
            ("failed", &n(Outcome::Failed)),
        ],
    ));
    if report.count(Outcome::Kept) > 0 {
        out.push(t("migrate.hintForce", &[]));
    }
    if report.dry_run {
        out.push(t("migrate.hintDryRun", &[]));
    }
    out.join("\n") + "\n"
}

/// Argumentos de `workboard migrate-v1` (sin el nombre del subcomando).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliArgs {
    pub from: Option<PathBuf>,
    pub to: Option<PathBuf>,
    pub preset: Option<String>,
    pub dry_run: bool,
    pub force: bool,
    pub help: bool,
}

pub fn parse_args(args: &[String]) -> ApiResult<CliArgs> {
    let mut out = CliArgs::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (a.as_str(), None),
        };
        let mut value = |name: &str| {
            inline
                .clone()
                .or_else(|| it.next().cloned())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| ApiError::new("migrate.error.missingValue", &[("flag", name)]))
        };
        match flag {
            "--from" => out.from = Some(PathBuf::from(value("--from")?)),
            "--to" => out.to = Some(PathBuf::from(value("--to")?)),
            "--preset" => out.preset = Some(value("--preset")?),
            "--dry-run" => out.dry_run = true,
            "--force" => out.force = true,
            "-h" | "--help" => out.help = true,
            other => return Err(ApiError::new("migrate.error.unknownArg", &[("arg", other)])),
        }
    }
    Ok(out)
}

/// `workboard migrate-v1 …` completo: interpreta los argumentos, migra con el almacén de credenciales del
/// sistema y DPAPI, y escribe el reporte. Devuelve el código de salida.
pub fn cli(args: &[String], out: &mut dyn io::Write) -> i32 {
    let to_default = wb_platform::config_dir();
    let lang_of = |dir: &Path| settings::load(&settings::path_in(dir)).lang.unwrap_or_else(|| V1_LANG.into());
    let print_err = |out: &mut dyn io::Write, lang: &str, e: &ApiError| {
        let params: Vec<(&str, &str)> = e.params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let _ = writeln!(out, "{}", wb_i18n::t(lang, &e.code, &params));
    };
    let parsed = match parse_args(args) {
        Ok(p) => p,
        Err(e) => {
            let lang = lang_of(&to_default);
            print_err(out, &lang, &e);
            let _ = writeln!(out, "{}", wb_i18n::t(&lang, "migrate.usage", &[]));
            return 2;
        }
    };
    let to = parsed.to.clone().unwrap_or(to_default);
    let lang = lang_of(&to);
    if parsed.help {
        let _ = writeln!(out, "{}", wb_i18n::t(&lang, "migrate.usage", &[]));
        return 0;
    }
    if !wb_platform::dpapi::SUPPORTED {
        let _ = writeln!(out, "{}", wb_i18n::t(&lang, "migrate.unsupported", &[]));
        return 0;
    }
    let Some(from) = parsed.from.clone().or_else(wb_platform::v1_config_dir) else {
        let _ = writeln!(out, "{}", wb_i18n::t(&lang, "migrate.unsupported", &[]));
        return 0;
    };
    let opts = Options {
        from,
        to,
        preset: parsed.preset.unwrap_or_else(|| DEFAULT_PRESET.into()),
        dry_run: parsed.dry_run,
        force: parsed.force,
    };
    let secrets = wb_platform::secrets::OsSecrets::from_env();
    match run(&opts, &secrets, &wb_platform::dpapi::unprotect) {
        Ok(report) => {
            let _ = write!(out, "{}", render(&report, &report.lang));
            i32::from(report.failed())
        }
        Err(e) => {
            print_err(out, &lang, &e);
            1
        }
    }
}
