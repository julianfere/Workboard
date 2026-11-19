//! Lógica de las herramientas del MCP `workboard`, separada del transporte (`main.rs`) para poder
//! probarla directo. Todo se lee en cada llamada: la app y el MCP escriben los mismos archivos.
//!
//! Modos (como la v1):
//! - `WORKBOARD_SESSION=<id>`: sesión lanzada por la app; escribe solo los items de esa sesión (un
//!   grupo de la sesión habilita sus items).
//! - `WORKBOARD_MCP_WRITE=1`: escritura en cualquier item de la raíz.
//! - ninguno: solo lectura.
//!
//! No expone git ni trackers. Cada escritura queda como evento (jsonl) en la carpeta de config.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{Value as Json, json};
use wb_core::error::CoreError;
use wb_core::frontmatter::Value;
use wb_core::notes::{Notes, item_key};
use wb_core::records::RunLog;
use wb_core::validate::{fmt_g, parse_date, parse_hours, validate_fields};
use wb_core::workspace::Item;
use wb_core::{Workflow, Workspace, py};

pub mod context;
pub mod tools;

pub const MAX_LIST: usize = 100;
pub const MAX_FILES: usize = 200;
pub const TRACKER_CACHE: &str = "tracker_cache.json";

/// Nombres de las herramientas (los usan los mensajes y la guía).
pub mod names {
    pub const SESSION: &str = "workboard_session";
    pub const LIST_ITEMS: &str = "workboard_list_items";
    pub const GET_ITEM: &str = "workboard_get_item";
    pub const UPDATE_ITEM: &str = "workboard_update_item";
    pub const ADD_TASK: &str = "workboard_add_task";
    pub const COMPLETE_TASK: &str = "workboard_complete_task";
    pub const MARK_RAN: &str = "workboard_mark_ran";
    pub const ADD_NOTE: &str = "workboard_add_note";
    pub const LIST_TOOLS: &str = "workboard_list_tools";
    pub const RUN_TOOL: &str = "workboard_run_tool";
}

/// Error de una herramienta: código traducible y parámetros (como `CoreError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    pub code: &'static str,
    pub params: Vec<(&'static str, String)>,
}

impl ToolError {
    fn new(code: &'static str, params: Vec<(&'static str, String)>) -> Self {
        Self { code, params }
    }

    pub fn render(&self, lang: &str) -> String {
        let params: Vec<(&str, &str)> = self.params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        wb_i18n::t(lang, self.code, &params)
    }
}

impl From<CoreError> for ToolError {
    fn from(e: CoreError) -> Self {
        Self { code: e.code, params: e.params }
    }
}

pub type ToolResult = Result<Json, ToolError>;

/// Entorno de una instancia del MCP.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub config_dir: PathBuf,
    pub session: Option<String>,
    pub write_all: bool,
    /// Raíz explícita (`WORKBOARD_ROOT`); si no, la de `settings.json`.
    pub root: Option<PathBuf>,
}

impl Ctx {
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        Self {
            config_dir: wb_platform::config_dir(),
            session: var("WORKBOARD_SESSION"),
            write_all: var("WORKBOARD_MCP_WRITE").as_deref() == Some("1"),
            root: var("WORKBOARD_ROOT").map(PathBuf::from),
        }
    }

    fn settings(&self) -> Json {
        std::fs::read_to_string(self.config_dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Json::Null)
    }

    /// Idioma de los textos para el agente (el de la app, o inglés).
    pub fn lang(&self) -> String {
        self.settings()["lang"].as_str().map_or(wb_i18n::DEFAULT_LANG, wb_i18n::resolve_lang).to_string()
    }

    pub fn workflow(&self) -> Workflow {
        let id = self.settings()["workflow"].as_str().unwrap_or("default").to_string();
        wb_plugins::resolve_workflow(&self.config_dir, &id)
            .unwrap_or_else(|_| Workflow::preset("default").expect("default preset"))
    }

    fn workspace(&self) -> Result<Workspace, ToolError> {
        let root = self
            .root
            .clone()
            .or_else(|| self.settings()["root"].as_str().map(PathBuf::from))
            .ok_or_else(|| ToolError::new("mcp.noRoot", vec![]))?;
        Ok(Workspace::open(&root, self.workflow(), Some(self.config_dir.join("backups")))?)
    }

    fn session_record(&self) -> Option<Json> {
        let id = self.session.as_deref()?;
        let mut regs: Vec<PathBuf> = std::fs::read_dir(self.config_dir.join("sessions"))
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        regs.sort();
        regs.iter().find_map(|reg| {
            let data: Json = serde_json::from_str(&std::fs::read_to_string(reg).ok()?).ok()?;
            let list = if data.is_array() { data.as_array()? } else { data["sessions"].as_array()? };
            list.iter().find(|s| s["id"] == id).cloned()
        })
    }

    /// Items de la sesión según el registro de la app; `None` fuera de una sesión.
    fn session_items(&self) -> Option<BTreeSet<String>> {
        self.session.as_ref()?;
        let rec = self.session_record().unwrap_or(Json::Null);
        let items = rec.get("items").or_else(|| rec.get("wis")).and_then(Json::as_array);
        Some(
            items
                .map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
        )
    }

    fn mode(&self) -> &'static str {
        if self.session.is_some() {
            "session"
        } else if self.write_all {
            "write"
        } else {
            "read-only"
        }
    }

    fn check_writable(&self, name: &str) -> Result<(), ToolError> {
        match self.session_items() {
            Some(items) => {
                if !in_session(name, &items) {
                    let list = if items.is_empty() {
                        wb_i18n::t(&self.lang(), "mcp.none", &[])
                    } else {
                        items.iter().cloned().collect::<Vec<_>>().join(", ")
                    };
                    return Err(ToolError::new(
                        "mcp.notInSession",
                        vec![("item", name.into()), ("items", list)],
                    ));
                }
                Ok(())
            }
            None if self.write_all => Ok(()),
            None => Err(ToolError::new("mcp.readOnly", vec![])),
        }
    }

    fn event(&self, kind: &str, item: &str, detail: &str) {
        let path = match &self.session {
            Some(id) => self.config_dir.join("sessions").join(format!("{id}.events.jsonl")),
            None => self.config_dir.join("mcp_events.jsonl"),
        };
        // Formato de la v1 (lo leen las métricas por sesión).
        let row = json!({
            "ts": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            "session": self.session,
            "tipo": kind,
            "wi": item,
            "detalle": detail,
        });
        let _ = (|| -> std::io::Result<()> {
            use std::io::Write;
            std::fs::create_dir_all(path.parent().unwrap())?;
            writeln!(std::fs::OpenOptions::new().create(true).append(true).open(&path)?, "{row}")
        })();
    }

    /// Prioridad, fecha, período y horas que dejó el último sync con un tracker (si hay).
    fn tracker_items(&self, ws: &Workspace) -> (String, serde_json::Map<String, Json>) {
        let cache: Json = std::fs::read_to_string(self.config_dir.join(TRACKER_CACHE))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Json::Null);
        let entry = &cache[RunLog::root_key(ws.root())];
        (
            entry["source"].as_str().unwrap_or("tracker").to_string(),
            entry["items"].as_object().cloned().unwrap_or_default(),
        )
    }
}

fn in_session(name: &str, items: &BTreeSet<String>) -> bool {
    items.contains(name) || items.contains(name.split('/').next().unwrap_or(name))
}

/// Ruta de un item (`grp/x`) con los separadores del sistema.
pub fn item_path(root: &Path, name: &str) -> PathBuf {
    name.split('/').fold(root.to_path_buf(), |p, part| p.join(part))
}

/// `str(value)` de Python para un valor del frontmatter (lista como `['a', 'b']`).
fn py_value(v: Option<&Value>) -> String {
    match v {
        None => String::new(),
        Some(Value::Text(s)) => s.clone(),
        Some(Value::List(l)) => format!("[{}]", l.iter().map(|s| py::repr(s)).collect::<Vec<_>>().join(", ")),
    }
}

struct Tools<'a> {
    ctx: &'a Ctx,
    ws: Workspace,
    lang: String,
}

impl<'a> Tools<'a> {
    fn new(ctx: &'a Ctx) -> Result<Self, ToolError> {
        Ok(Self { ws: ctx.workspace()?, lang: ctx.lang(), ctx })
    }

    fn wf(&self) -> &Workflow {
        self.ws.workflow()
    }

    fn meta_text(&self, it: &Item, key: &str) -> Json {
        it.meta.get(key).map_or(json!(""), |v| serde_json::to_value(v).unwrap())
    }

    /// `item` = nombre de carpeta o id de work item.
    fn resolve(&self, item: &str) -> Result<Item, ToolError> {
        let key = py::strip(item).trim_start_matches('#').to_string();
        let items = self.ws.list_items()?;
        let exact: Vec<&Item> = items.iter().filter(|it| it.name == key).collect();
        let found =
            if exact.is_empty() { items.iter().filter(|it| it.ids.contains(&key)).collect() } else { exact };
        match found.as_slice() {
            [] => Err(ToolError::new(
                "mcp.itemNotFound",
                vec![("item", py::repr(&key)), ("tool", names::LIST_ITEMS.into())],
            )),
            [one] => Ok((*one).clone()),
            many => Err(ToolError::new(
                "mcp.itemAmbiguous",
                vec![
                    ("item", key),
                    ("folders", many.iter().map(|it| it.name.as_str()).collect::<Vec<_>>().join(", ")),
                ],
            )),
        }
    }

    fn planning(&self, it: &Item, out: &mut serde_json::Map<String, Json>) {
        let (source, cache) = self.ctx.tracker_items(&self.ws);
        let keys = &self.wf().fields;
        let tracked: Vec<&Json> = it.ids.iter().filter_map(|id| cache.get(id)).collect();
        let text = |k: &str| it.meta.get(k).and_then(Value::as_text).unwrap_or("").to_string();
        let from =
            |v: String| wb_i18n::t(&self.lang, "mcp.fromTracker", &[("value", &v), ("source", &source)]);

        let prios: Vec<&Json> =
            tracked.iter().map(|a| &a["priority"]).filter(|p| !p.is_null() && *p != "").collect();
        if let Some(min) = prios.iter().filter_map(|p| p.as_f64()).reduce(f64::min) {
            out.insert(
                "priority".into(),
                prios.iter().find(|p| p.as_f64() == Some(min)).map_or(json!(min), |p| (*p).clone()),
            );
        }
        let own_due = text(&keys.due);
        let tracker_due = tracked
            .iter()
            .filter_map(|a| a["due"].as_str().filter(|s| !s.is_empty()))
            .map(|s| s.chars().take(10).collect::<String>())
            .min()
            .unwrap_or_default();
        let due = if own_due.is_empty() { tracker_due } else { own_due.clone() };
        if !due.is_empty() {
            out.insert("due".into(), json!(if own_due.is_empty() { from(due.clone()) } else { due.clone() }));
        }
        let mut period = text(&keys.period);
        if period.is_empty()
            && let Some((y, m, d)) = parse_date(&due.chars().take(10).collect::<String>())
        {
            period = self.wf().period_key(y, m, d);
        }
        if !period.is_empty() {
            let start = self.wf().period.start_day;
            let span = if start == 1 {
                wb_i18n::t(&self.lang, "mcp.periodMonth", &[("period", &period)])
            } else {
                let end = (start - 1).to_string();
                wb_i18n::t(
                    &self.lang,
                    "mcp.periodSpan",
                    &[("period", &period), ("start", &start.to_string()), ("end", &end)],
                )
            };
            out.insert("period".into(), json!(span));
        }
        let own_hours = Some(text(&keys.hours)).filter(|h| !h.is_empty()).and_then(|h| parse_hours(&h));
        match own_hours {
            Some(h) => {
                out.insert("hours".into(), json!(fmt_g(h)));
            }
            None => {
                let est: Vec<f64> =
                    tracked.iter().filter_map(|a| a["estimate"].as_f64()).filter(|e| *e > 0.0).collect();
                if !est.is_empty() {
                    out.insert("hours".into(), json!(from(fmt_g(est.iter().sum()))));
                }
            }
        }
    }

    fn summary(&self, it: &Item) -> Json {
        let keys = &self.wf().fields;
        let mut out = serde_json::Map::new();
        out.insert("folder".into(), json!(it.name));
        out.insert("path".into(), json!(item_path(self.ws.root(), &it.name).display().to_string()));
        out.insert("ids".into(), json!(it.ids));
        out.insert("title".into(), json!(it.title));
        out.insert("status".into(), self.meta_text(it, &keys.status));
        out.insert("type".into(), json!(it.item_type));
        out.insert(
            "environments".into(),
            it.meta.get(&keys.environments).map_or(json!([]), |v| serde_json::to_value(v).unwrap()),
        );
        out.insert("branch".into(), self.meta_text(it, &keys.branch));
        out.insert("notes".into(), self.meta_text(it, &keys.notes));
        out.insert("open_tasks".into(), json!(it.tasks.iter().filter(|t| !t.done).count()));
        out.insert(
            "readme".into(),
            json!(item_path(self.ws.root(), &it.name).join("README.md").display().to_string()),
        );
        self.planning(it, &mut out);
        if it.kind == wb_core::workspace::ItemKind::Group {
            out.insert("group_of".into(), json!(it.members));
        } else if !it.group.is_empty() {
            out.insert("group".into(), json!(it.group));
        }
        Json::Object(out)
    }

    fn task_list(it: &Item) -> Json {
        json!(
            it.tasks
                .iter()
                .enumerate()
                .map(|(i, t)| json!({"index": i, "text": t.text, "done": t.done}))
                .collect::<Vec<_>>()
        )
    }
}

/// Texto de la guía: qué es Workboard y cómo trabajar con él.
pub fn guide(ctx: &Ctx) -> String {
    context::guide_for(&ctx.workflow(), &ctx.lang())
}

/// `instructions` del MCP: dentro de una sesión, el mismo contexto que recibe el agente (sus items, la
/// guía y las tools); fuera de una sesión, solo la guía.
pub fn instructions(ctx: &Ctx) -> String {
    let (Some(rec), Ok(ws)) = (ctx.session_record(), ctx.workspace()) else { return guide(ctx) };
    let Ok(all) = ws.list_items() else { return guide(ctx) };
    let items: Vec<String> = rec
        .get("items")
        .or_else(|| rec.get("wis"))
        .and_then(Json::as_array)
        .map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let name = rec["name"].as_str().or_else(|| rec["nombre"].as_str()).unwrap_or_default();
    context::session_context(
        ws.workflow(),
        &ctx.lang(),
        ws.root(),
        name,
        &items,
        &all,
        &registered_tools(ctx),
    )
}

// ---------- herramientas ----------

pub fn session(ctx: &Ctx) -> ToolResult {
    let t = Tools::new(ctx)?;
    let mut out = serde_json::Map::new();
    out.insert("mode".into(), json!(ctx.mode()));
    out.insert("root".into(), json!(t.ws.root().display().to_string()));
    if let Some(items) = ctx.session_items() {
        let list: Vec<Json> =
            t.ws.list_items()?
                .iter()
                .filter(|it| in_session(&it.name, &items))
                .map(|it| t.summary(it))
                .collect();
        out.insert("items".into(), json!(list));
    }
    out.insert("instructions".into(), json!(guide(ctx)));
    Ok(Json::Object(out))
}

pub fn list_items(ctx: &Ctx, search: &str, status: &str, include_closed: bool) -> ToolResult {
    let t = Tools::new(ctx)?;
    let q = py::strip(search).to_lowercase();
    let keys = &t.wf().fields;
    let mut items = t.ws.list_items()?;
    items.sort_by(|a, b| b.last.total_cmp(&a.last));
    let mut out = Vec::new();
    for it in &items {
        let st = it.meta.get(&keys.status).and_then(Value::as_text).unwrap_or("");
        if !status.is_empty() && st != status {
            continue;
        }
        if !include_closed && status.is_empty() && t.wf().is_closed(st) {
            continue;
        }
        let hay = [it.name.clone(), it.title.clone(), it.ids.join(" "), py_value(it.meta.get(&keys.notes))]
            .join(" ")
            .to_lowercase();
        if !q.is_empty() && !hay.contains(&q) {
            continue;
        }
        out.push(t.summary(it));
    }
    let total = out.len();
    out.truncate(MAX_LIST);
    Ok(json!({"total": total, "items": out}))
}

pub fn get_item(ctx: &Ctx, item: &str) -> ToolResult {
    let t = Tools::new(ctx)?;
    let it = t.resolve(item)?;
    let mut out = t.summary(&it);
    let runs = RunLog::new(ctx.config_dir.join("runs.json")).of_root(t.ws.root());
    let mut files = it.files.clone();
    files.sort_by(|a, b| b.mtime.total_cmp(&a.mtime));
    let runnable: BTreeSet<&str> =
        t.wf().categories().filter(|c| c.runnable).map(|c| c.id.as_str()).collect();
    let files: Vec<Json> = files
        .iter()
        .take(MAX_FILES)
        .map(|f| {
            let mut e = json!({"path": f.path, "category": f.cat});
            if runnable.contains(f.cat.as_str()) {
                let marks: serde_json::Map<String, Json> = runs
                    .get(&f.path)
                    .map(|m| {
                        m.iter()
                            .map(|(env, ts)| {
                                let when = chrono::DateTime::from_timestamp(*ts as i64, 0)
                                    .map(|d| {
                                        d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()
                                    })
                                    .unwrap_or_default();
                                (env.clone(), json!(when))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                e["ran"] = Json::Object(marks);
            }
            e
        })
        .collect();
    let o = out.as_object_mut().unwrap();
    o.insert("frontmatter".into(), serde_json::to_value(&it.meta).unwrap());
    o.insert("tasks".into(), Tools::task_list(&it));
    o.insert("files".into(), json!(files));
    o.insert("deploy".into(), json!(it.deploy));
    Ok(out)
}

/// Campos que cambia `update_item` (los no pasados quedan igual; "" borra).
#[derive(Debug, Default, Clone)]
pub struct ItemUpdate {
    pub status: Option<String>,
    pub environments: Option<Vec<String>>,
    pub notes: Option<String>,
    pub branch: Option<String>,
    pub item_type: Option<String>,
    pub due: Option<String>,
    pub hours: Option<String>,
}

pub fn update_item(ctx: &Ctx, item: &str, upd: &ItemUpdate) -> ToolResult {
    let t = Tools::new(ctx)?;
    let it = t.resolve(item)?;
    ctx.check_writable(&it.name)?;
    let k = &t.wf().fields;
    let s = |v: &Option<String>| v.as_ref().map_or(Json::Null, |x| json!(x));
    let raw: IndexMap<String, Json> = [
        (k.status.clone(), s(&upd.status)),
        (k.environments.clone(), upd.environments.as_ref().map_or(Json::Null, |e| json!(e))),
        (k.notes.clone(), s(&upd.notes)),
        (k.branch.clone(), s(&upd.branch)),
        (k.kind.clone(), s(&upd.item_type)),
        (k.due.clone(), s(&upd.due)),
        (k.hours.clone(), s(&upd.hours)),
    ]
    .into_iter()
    .collect();
    let fields = validate_fields(t.wf(), &raw)?;
    if fields.is_empty() {
        return Err(ToolError::new("mcp.noFields", vec![]));
    }
    let new = t.ws.update_meta(&it.name, &fields)?;
    let dash = |s: String| if s.is_empty() || s == "[]" { "—".to_string() } else { s };
    let detail = fields
        .iter()
        .map(|(key, v)| format!("{key}: {} → {}", dash(py_value(it.meta.get(key))), dash(py_value(Some(v)))))
        .collect::<Vec<_>>()
        .join(", ");
    ctx.event("set_estado", &it.name, &detail);
    Ok(t.summary(&new))
}

pub fn add_task(ctx: &Ctx, item: &str, text: &str) -> ToolResult {
    let t = Tools::new(ctx)?;
    let it = t.resolve(item)?;
    ctx.check_writable(&it.name)?;
    let new = t.ws.add_task(&it.name, text)?;
    ctx.event("add_task", &it.name, &py::split_ws(text).collect::<Vec<_>>().join(" "));
    Ok(json!({"folder": new.name, "tasks": Tools::task_list(&new)}))
}

pub fn complete_task(ctx: &Ctx, item: &str, task: &str, done: bool) -> ToolResult {
    let t = Tools::new(ctx)?;
    let it = t.resolve(item)?;
    ctx.check_writable(&it.name)?;
    let tasks = &it.tasks;
    let key = py::strip(task);
    let by_index =
        key.parse::<usize>().ok().filter(|i| key.chars().all(|c| c.is_ascii_digit()) && *i < tasks.len());
    let idx = match by_index.or_else(|| tasks.iter().position(|t| t.text == key)) {
        Some(i) => i,
        None => {
            let lower = key.to_lowercase();
            let hits: Vec<usize> =
                (0..tasks.len()).filter(|&i| tasks[i].text.to_lowercase().contains(&lower)).collect();
            if hits.len() != 1 {
                let list =
                    format!("[{}]", tasks.iter().map(|t| py::repr(&t.text)).collect::<Vec<_>>().join(", "));
                let code = if hits.is_empty() { "mcp.task.none" } else { "mcp.task.many" };
                return Err(ToolError::new(code, vec![("task", py::repr(key)), ("tasks", list)]));
            }
            hits[0]
        }
    };
    let new = t.ws.set_task(&it.name, idx as i64, done, Some(&tasks[idx].text))?;
    ctx.event(if done { "complete_task" } else { "reopen_task" }, &it.name, &tasks[idx].text);
    Ok(json!({"folder": new.name, "tasks": Tools::task_list(&new)}))
}

pub fn mark_ran(ctx: &Ctx, item: &str, file: &str, environment: &str, ran: bool) -> ToolResult {
    let t = Tools::new(ctx)?;
    let it = t.resolve(item)?;
    ctx.check_writable(&it.name)?;
    let key = py::strip(file).replace('\\', "/").to_lowercase();
    let hits: Vec<_> = it
        .files
        .iter()
        .filter(|f| {
            let p = f.path.to_lowercase();
            p == key || p.ends_with(&format!("/{key}"))
        })
        .collect();
    if hits.len() != 1 {
        let code = if hits.is_empty() { "mcp.file.none" } else { "mcp.file.many" };
        return Err(ToolError::new(code, vec![("item", it.name.clone()), ("file", py::repr(file))]));
    }
    let f = hits[0];
    let env = py::strip(environment).to_lowercase();
    let marks =
        RunLog::new(ctx.config_dir.join("runs.json")).mark(t.wf(), t.ws.root(), &f.path, &env, ran, None)?;
    ctx.event(if ran { "run" } else { "unrun" }, &it.name, &format!("{} · {environment}", f.path));
    Ok(json!({"file": f.path, "ran": marks.keys().collect::<Vec<_>>()}))
}

pub fn add_note(ctx: &Ctx, item: &str, title: &str, text: &str) -> ToolResult {
    let t = Tools::new(ctx)?;
    let it = t.resolve(item)?;
    ctx.check_writable(&it.name)?;
    let folder =
        ctx.settings()["notesFolder"].as_str().map_or_else(|| ctx.config_dir.join("notes"), PathBuf::from);
    let store = Notes::new(folder, t.wf());
    let rec = ctx.session_record().unwrap_or(Json::Null);
    let author = rec.get("agent").or_else(|| rec.get("agente")).and_then(Json::as_str).unwrap_or("claude");
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let note = store.create(title, &[item_key(&it)], author, text, &today)?;
    ctx.event("add_note", &it.name, &note.id);
    Ok(json!({"note": note.id, "folder": store.folder().display().to_string(), "ids": note.items}))
}

/// Tools promovidas por el usuario (`<config>/tools/<nombre>/tool.json`, formato de la v1).
pub fn registered_tools(ctx: &Ctx) -> Vec<Json> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(ctx.config_dir.join("tools"))
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.join("tool.json").is_file()).collect())
        .unwrap_or_default();
    dirs.sort();
    dirs.iter()
        .filter_map(|d| {
            serde_json::from_str::<Json>(&std::fs::read_to_string(d.join("tool.json")).ok()?).ok()
        })
        .collect()
}

pub fn list_tools(ctx: &Ctx) -> ToolResult {
    let tools: Vec<Json> = tools::list(&ctx.config_dir)
        .iter()
        .map(|t| {
            json!({"name": t.meta.name, "description": t.meta.description, "usage": t.meta.usage,
                   "interpreter": t.meta.interpreter, "ok": t.ok})
        })
        .collect();
    Ok(json!({"tools": tools}))
}

/// Corre una tool en la carpeta del item (`item`, o el primero de la sesión). Solo dentro de una sesión
/// y solo si el script sigue igual que cuando se promovió.
pub fn run_tool(ctx: &Ctx, name: &str, args: &[String], item: &str) -> ToolResult {
    if ctx.session.is_none() {
        return Err(ToolError::new("mcp.tool.sessionOnly", vec![]));
    }
    let t = Tools::new(ctx)?;
    let items = ctx.session_items().unwrap_or_default();
    let folder = if !item.is_empty() {
        let it = t.resolve(item)?;
        ctx.check_writable(&it.name)?;
        it.name
    } else {
        items.iter().next().cloned().ok_or_else(|| ToolError::new("mcp.tool.noItem", vec![]))?
    };
    let path = t.ws.folder_of(&folder)?;
    if tools::get(&ctx.config_dir, name).is_none() {
        return Err(ToolError::new(
            "mcp.tool.notFound",
            vec![("name", name.into()), ("tool", names::LIST_TOOLS.into())],
        ));
    }
    let out = tools::run(&ctx.config_dir, name, args, &path, tools::RUN_TIMEOUT)?;
    ctx.event("run_tool", &folder, name);
    Ok(out)
}
