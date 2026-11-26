//! API de la app: un único `dispatch(cmd, args)` que usan la ventana (Tauri, comando `api`) y el servidor
//! de desarrollo (`wb-devserver`, `POST /api/<cmd>`). Cada comando es un wrapper fino sobre `wb-core`.
//! Todo se lee en cada llamada: la carpeta de trabajos puede cambiar por fuera (agentes, el usuario).

mod alerts;
mod bulk;
mod detail;
pub mod errlog;
pub mod error;
mod git;
pub mod migrate;
mod notes;
mod plugins;
mod prefs;
mod sessions;
pub mod settings;
mod stats;
mod tracker;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Value as Json, json};
use wb_core::frontmatter::{Meta, Value};
use wb_core::notes::Notes;
use wb_core::records::{Inbox, RunLog};
use wb_core::text::slugify;
use wb_core::validate::validate_fields;
use wb_core::workspace::{Item, ItemKind};
use wb_core::write::Change;
use wb_core::{StateCategory, Workflow, Workspace};

pub use error::{ApiError, ApiResult};
pub use sessions::{HOOK_BIN_ENV, MCP_BIN_ENV};
use settings::Settings;

pub struct Api {
    config_dir: PathBuf,
    settings: Mutex<Settings>,
    sessions: OnceLock<Arc<wb_sessions::SessionManager>>,
    /// Ejecutables del MCP y del hook, si no son los que viajan junto a la app.
    session_bins: Mutex<Option<(PathBuf, PathBuf)>>,
    /// Almacén de secretos y transporte HTTP de los trackers.
    tracker: tracker::TrackerDeps,
    git: git::GitState,
    /// Serializa las escrituras de `ui_prefs.json`.
    prefs_lock: Mutex<()>,
    /// Archivos exportados con «Guardar como» en esta ejecución (los únicos fuera de Descargas que se muestran).
    exports: Mutex<Vec<PathBuf>>,
}

fn to_json<T: Serialize>(v: T) -> ApiResult<Json> {
    serde_json::to_value(v).map_err(ApiError::generic)
}

/// Argumentos de un comando (`{nombre: valor}`).
pub(crate) struct Args<'a>(&'a Json);

impl Args<'_> {
    fn get(&self, k: &str) -> &Json {
        &self.0[k]
    }

    fn str(&self, k: &str) -> ApiResult<&str> {
        self.0[k].as_str().ok_or_else(|| ApiError::new("error.badArgument", &[("name", k)]))
    }

    fn opt_str(&self, k: &str) -> Option<&str> {
        self.0[k].as_str()
    }

    fn int(&self, k: &str) -> ApiResult<i64> {
        self.0[k].as_i64().ok_or_else(|| ApiError::new("error.badArgument", &[("name", k)]))
    }

    fn bool(&self, k: &str) -> bool {
        self.0[k].as_bool().unwrap_or(false)
    }
}

impl Api {
    pub fn new(config_dir: PathBuf) -> Self {
        let mut s = settings::load(&settings::path_in(&config_dir));
        if wb_plugins::resolve_workflow(&config_dir, &s.workflow).is_err() {
            s.workflow = Settings::default().workflow;
        }
        Self {
            config_dir,
            settings: Mutex::new(s),
            sessions: OnceLock::new(),
            session_bins: Mutex::new(None),
            tracker: Default::default(),
            git: Default::default(),
            prefs_lock: Mutex::new(()),
            exports: Mutex::new(Vec::new()),
        }
    }

    fn settings(&self) -> Settings {
        self.settings.lock().expect("settings lock").clone()
    }

    fn save_settings(&self, s: Settings) -> ApiResult<Settings> {
        settings::save(&settings::path_in(&self.config_dir), &s)?;
        *self.settings.lock().expect("settings lock") = s.clone();
        Ok(s)
    }

    fn workflow(&self) -> ApiResult<Workflow> {
        Ok(wb_plugins::resolve_workflow(&self.config_dir, &self.settings().workflow)?)
    }

    fn workspace(&self) -> ApiResult<Workspace> {
        let root = self.settings().root.ok_or_else(|| ApiError::new("error.noRoot", &[]))?;
        Ok(Workspace::open(Path::new(&root), self.workflow()?, Some(self.config_dir.join("backups")))?)
    }

    fn notes(&self) -> ApiResult<Notes> {
        let folder =
            self.settings().notes_folder.map_or_else(|| self.config_dir.join("notes"), PathBuf::from);
        Ok(Notes::new(folder, &self.workflow()?))
    }

    fn inbox(&self) -> ApiResult<Inbox> {
        Ok(Inbox::new(self.config_dir.join("inbox.md"), &self.workflow()?))
    }

    fn runs(&self) -> RunLog {
        RunLog::new(self.config_dir.join("runs.json"))
    }

    /// Ejecuta un comando. Los errores siempre vuelven como `ApiError` (nunca un panic ni texto armado) y
    /// quedan en `<config>/workboard.log`.
    pub fn dispatch(&self, cmd: &str, args: &Json) -> ApiResult<Json> {
        let out = self.run_command(cmd, args);
        if let Err(e) = &out {
            errlog::write(&self.config_dir, cmd, e);
        }
        out
    }

    fn run_command(&self, cmd: &str, args: &Json) -> ApiResult<Json> {
        let a = Args(args);
        match cmd {
            // ---------- app ----------
            "app_info" => Ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "os": std::env::consts::OS,
                "configDir": self.config_dir.display().to_string(),
            })),
            "workflow_presets" => {
                let presets: ApiResult<Vec<Json>> = Workflow::preset_ids()
                    .map(|id| Ok(json!({"id": id, "name": Workflow::preset(id)?.name})))
                    .collect();
                let mut presets = presets?;
                for (plugin, w) in wb_plugins::Plugins::load(&self.config_dir).presets() {
                    presets.push(json!({"id": w.id, "name": w.name, "plugin": plugin}));
                }
                Ok(Json::Array(presets))
            }
            "workflow_preset" => to_json(wb_plugins::resolve_workflow(&self.config_dir, a.str("id")?)?),
            "workflow" => to_json(self.workflow()?),
            "get_settings" => to_json(self.settings()),
            "set_settings" => {
                let s: Settings =
                    serde_json::from_value(a.get("settings").clone()).map_err(ApiError::generic)?;
                wb_plugins::resolve_workflow(&self.config_dir, &s.workflow)?;
                if let Some(lang) = &s.lang
                    && !wb_i18n::LANGS.contains(&lang.as_str())
                {
                    return Err(ApiError::new("error.badArgument", &[("name", "lang")]));
                }
                to_json(self.save_settings(s)?)
            }
            "open_root" => {
                let path = a.str("path")?;
                let ws = Workspace::open(Path::new(path), self.workflow()?, None)?;
                let mut s = self.settings();
                s.open_root(&ws.root().display().to_string());
                to_json(self.save_settings(s)?)
            }
            "forget_recent" => {
                let mut s = self.settings();
                s.forget_recent(a.str("path")?);
                to_json(self.save_settings(s)?)
            }
            "restore_recent" => {
                let mut s = self.settings();
                s.restore_recent(a.str("path")?, usize::try_from(a.int("pos")?).unwrap_or(0));
                to_json(self.save_settings(s)?)
            }
            "recent_folders" => {
                let s = self.settings();
                let list: Vec<Json> = s
                    .recents
                    .iter()
                    .map(|r| {
                        json!({
                            "path": r,
                            "name": s.display_name(r),
                            "alias": s.alias_of(r),
                            "exists": Path::new(r).is_dir(),
                            "current": s.root.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(r)),
                        })
                    })
                    .collect();
                Ok(Json::Array(list))
            }
            "set_alias" => {
                let path = a.str("path")?;
                let path =
                    dunce::canonicalize(path).map_or_else(|_| path.to_string(), |p| p.display().to_string());
                let mut s = self.settings();
                s.set_alias(&path, a.opt_str("name").unwrap_or_default())
                    .map_err(|max| ApiError::new("projects.aliasTooLong", &[("max", &max.to_string())]))?;
                to_json(self.save_settings(s)?)
            }

            // ---------- carpeta de trabajos ----------
            "signature" => to_json(self.workspace()?.signature()),
            "list_items" => {
                let ws = self.workspace()?;
                Ok(json!({
                    "root": ws.root().display().to_string(),
                    "items": ws.list_items()?,
                    "deleted": ws.list_deleted()?.len(),
                    "signature": ws.signature(),
                }))
            }
            "get_item" => {
                let ws = self.workspace()?;
                let folder = ws.folder_of(a.str("name")?)?;
                let item = ws.build_item(&folder)?;
                let readme = folder.join(wb_core::workspace::README);
                let text = if readme.exists() { wb_core::frontmatter::read(&readme)? } else { String::new() };
                // Un grupo muestra también los archivos de sus items: van sus marcas.
                let prefix = format!("{}/", item.name);
                let runs: serde_json::Map<String, Json> = self
                    .runs()
                    .of_root(ws.root())
                    .into_iter()
                    .filter(|(p, _)| p.starts_with(&prefix))
                    .map(|(p, m)| (p, json!(m)))
                    .collect();
                Ok(json!({"item": item, "readme": text, "runs": runs}))
            }
            "update_fields" => {
                let ws = self.workspace()?;
                let raw: IndexMap<String, Json> = a
                    .get("fields")
                    .as_object()
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                let fields = validate_fields(ws.workflow(), &raw)?;
                to_json(ws.update_meta(a.str("name")?, &fields)?)
            }
            "add_task" => to_json(self.workspace()?.add_task(a.str("name")?, a.str("text")?)?),
            "set_task" => to_json(self.workspace()?.set_task(
                a.str("name")?,
                a.int("index")?,
                a.bool("done"),
                a.opt_str("expect"),
            )?),
            "edit_task" => to_json(self.workspace()?.edit_task(
                a.str("name")?,
                a.int("index")?,
                a.str("text")?,
                a.opt_str("expect"),
            )?),
            "remove_task" => to_json(self.workspace()?.remove_task(
                a.str("name")?,
                a.int("index")?,
                a.opt_str("expect"),
            )?),
            "move_task" => self.move_task(
                a.opt_str("src").unwrap_or(""),
                a.int("index")?,
                a.str("expect")?,
                a.opt_str("dst").unwrap_or(""),
            ),
            "read_file" => to_json(self.workspace()?.read_file(a.str("path")?)?),
            "read_media" => to_json(self.workspace()?.read_media(a.str("path")?)?),
            "list_versions" => to_json(self.workspace()?.list_versions(a.str("name")?)?),
            "read_version" => to_json(self.workspace()?.read_version(a.str("name")?, a.str("id")?)?),
            "restore_version" => to_json(self.workspace()?.restore_version(a.str("name")?, a.str("id")?)?),
            "create_item" => self.create_item(&a),
            "create_group" => self.create_group(a.str("title")?, a.get("members")),
            "move_folder" => {
                self.moved(a.str("name")?, |ws| Ok(ws.move_folder(a.str("name")?, a.str("dest")?)?))
            }
            "rename_folder" => {
                self.moved(a.str("name")?, |ws| Ok(ws.rename_folder(a.str("name")?, a.str("base")?)?))
            }
            "trash" => self.moved(a.str("name")?, |ws| Ok(ws.trash(a.str("name")?)?)),
            "untrash" => {
                self.moved(a.str("deleted")?, |ws| Ok(ws.untrash(a.str("deleted")?, a.str("name")?)?))
            }
            "list_deleted" => to_json(self.workspace()?.list_deleted()?),
            "mark_ran" => {
                let ws = self.workspace()?;
                let rel = a.str("path")?;
                // Como la v1: solo archivos que existen dentro de la raíz.
                if !ws.safe_path(rel)?.is_file() {
                    return Err(ApiError::new("core.ws.fileNotFound", &[("path", rel)]));
                }
                let marks =
                    self.runs().mark(ws.workflow(), ws.root(), rel, a.str("env")?, a.bool("on"), None)?;
                to_json(marks)
            }
            "reveal" | "open_path" => {
                let ws = self.workspace()?;
                let p = ws.safe_path(a.opt_str("path").unwrap_or(""))?;
                if cmd == "reveal" {
                    wb_platform::reveal_path(&p)?
                } else {
                    wb_platform::open_path(&p)?
                }
                Ok(Json::Null)
            }

            // ---------- bandeja y notas ----------
            "inbox" => to_json(self.inbox()?.tasks()?),
            "inbox_add" => to_json(self.inbox()?.add(a.str("text")?)?),
            "inbox_set" => {
                to_json(self.inbox()?.set(a.int("index")?, a.bool("done"), a.opt_str("expect"))?)
            }
            "inbox_edit" => {
                to_json(self.inbox()?.edit(a.int("index")?, a.str("text")?, a.opt_str("expect"))?)
            }
            "inbox_remove" => to_json(self.inbox()?.remove(a.int("index")?, a.opt_str("expect"))?),
            "inbox_clear_done" => to_json(self.inbox()?.clear_done()?),
            "notes" => to_json(self.notes()?.list()),
            "note_read" => to_json(self.notes()?.read(a.str("id")?)?),
            "note_save" => {
                to_json(self.notes()?.save(a.str("id")?, a.str("body")?, a.get("mtime").as_f64())?)
            }
            "note_create" => {
                let items: Vec<String> = a
                    .get("items")
                    .as_array()
                    .map_or(vec![], |l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect());
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                to_json(self.notes()?.create(
                    a.str("title")?,
                    &items,
                    "",
                    a.opt_str("body").unwrap_or(""),
                    &today,
                )?)
            }
            _ => self
                .session_command(cmd, &a)
                .or_else(|| self.tracker_command(cmd, &a))
                .or_else(|| self.plugin_command(cmd, &a))
                .or_else(|| self.stats_command(cmd, &a))
                .or_else(|| self.git_command(cmd, &a))
                .or_else(|| self.alerts_command(cmd, &a))
                .or_else(|| self.notes_command(cmd, &a))
                .or_else(|| self.detail_command(cmd, &a))
                .or_else(|| self.bulk_command(cmd, &a))
                .or_else(|| self.prefs_command(cmd, &a))
                .unwrap_or_else(|| Err(ApiError::new("error.unknownCommand", &[("name", cmd)]))),
        }
    }

    /// Después de mover una carpeta, las marcas de ejecutados la siguen.
    fn moved(&self, old: &str, f: impl FnOnce(&Workspace) -> ApiResult<String>) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let new = f(&ws)?;
        if new != old {
            self.follow_move(&ws, old, &new)?;
        }
        Ok(json!(new))
    }

    /// Lo que referencia una carpeta por nombre la sigue: marcas de ejecutados, sesiones y notas vinculadas
    /// por carpeta (no a eliminados: ahí sesiones y notas quedan como estaban, para restaurar).
    fn follow_move(&self, ws: &Workspace, old: &str, new: &str) -> ApiResult<()> {
        self.runs().move_prefix(ws.root(), old, new)?;
        self.sessions_moved(ws, old, new);
        if !ws.workflow().is_ignored_folder(new.split('/').next().unwrap_or(new))
            && let Ok(notes) = self.notes()
        {
            notes.rename_links(old, new);
        }
        Ok(())
    }

    /// Mueve una tarea entre items y la bandeja ("" = bandeja). Primero la agrega en el destino (con su
    /// tildado) y después la borra del origen: si algo falla queda duplicada, nunca perdida. Los README se
    /// escriben solo con `add_task`/`set_task`/`remove_task` (el escritor único).
    fn move_task(&self, src: &str, index: i64, expect: &str, dst: &str) -> ApiResult<Json> {
        if src == dst {
            return Ok(Json::Null);
        }
        let ws = self.workspace()?;
        let inbox = self.inbox()?;
        let tasks = if src.is_empty() { inbox.tasks()? } else { ws.build_item(&ws.folder_of(src)?)?.tasks };
        let task = usize::try_from(index)
            .ok()
            .and_then(|i| tasks.get(i))
            .ok_or_else(|| ApiError::new("core.task.notFound", &[]))?;
        if task.text != expect.trim() {
            return Err(ApiError::new("core.task.changed", &[]));
        }
        let (text, done) = (task.text.clone(), task.done);
        if dst.is_empty() {
            let after = inbox.add(&text)?;
            if done {
                inbox.set(after.len() as i64 - 1, true, Some(&text))?;
            }
        } else {
            let after = ws.add_task(dst, &text)?;
            if done {
                ws.set_task(dst, after.tasks.len() as i64 - 1, true, Some(&text))?;
            }
        }
        if src.is_empty() {
            inbox.remove(index, Some(&text))?;
        } else {
            ws.remove_task(src, index, Some(&text))?;
        }
        Ok(json!(true))
    }

    /// Item nuevo sin id del tracker (un arreglo del proyecto): título obligatorio, tipo por defecto el
    /// del preset (`newItem.defaultType`) y carpeta `newItem.folderTemplate`. Con `plan`, queda en el
    /// estado de planificación y se abre la sesión de Planear; si no, en el estado inicial.
    /// Devuelve `{item, session}`.
    fn create_item(&self, a: &Args) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let wf = ws.workflow();
        let title = squash(a.opt_str("title").unwrap_or(""));
        if title.is_empty() {
            return Err(ApiError::new("newItem.titleRequired", &[]));
        }
        let item_type = match a.opt_str("type").map(str::trim).unwrap_or("") {
            "" => wf.new_item.default_type.clone(),
            t => t.to_string(),
        };
        if !item_type.is_empty() && !wf.types.iter().any(|t| t.id == item_type) {
            return Err(ApiError::new("error.badArgument", &[("name", "type")]));
        }
        let folder = match a.opt_str("folder").map(str::trim).unwrap_or("") {
            "" => folder_without_id(wf, &item_type, &title),
            f => f.to_string(),
        };
        let new = NewItem {
            id: String::new(),
            title,
            local_type: item_type,
            folder,
            description: a.opt_str("description").unwrap_or("").trim().to_string(),
            criteria: a.opt_str("criteria").unwrap_or("").trim().to_string(),
            tracker_type: String::new(),
            tracker_state: String::new(),
            source: String::new(),
        };
        self.create_new_item(&ws, &new, a.bool("plan").then_some(a))
    }

    /// Escribe el README de un item nuevo (con o sin id) y, con `plan`, abre la sesión de Planear
    /// (`plan_item`, con `cols`/`rows` de los argumentos). Devuelve `{item, session}`.
    pub(crate) fn create_new_item(
        &self,
        ws: &Workspace,
        d: &NewItem,
        plan: Option<&Args>,
    ) -> ApiResult<Json> {
        let wf = ws.workflow();
        let category = if plan.is_some() { StateCategory::Planned } else { StateCategory::Backlog };
        let state = match wf.states.iter().find(|s| s.category == category) {
            Some(s) => s.id.clone(),
            None if plan.is_some() => return Err(ApiError::new("error.noPlanState", &[])),
            None => wf.states[0].id.clone(),
        };
        let path = ws.create_folder(&d.folder)?;
        let name = ws.rel(&path);
        let mut meta = Meta::new();
        meta.insert(wf.fields.status.clone(), Value::Text(state));
        if !d.id.is_empty() {
            meta.insert(wf.fields.item_ids.clone(), Value::Text(d.id.clone()));
        }
        if !d.local_type.is_empty() {
            meta.insert(wf.fields.kind.clone(), Value::Text(d.local_type.clone()));
        }
        let body = new_item_body(wf, &name, d);
        let item = ws.write_readme(&name, |_, _| Ok(Change::Parts(meta.clone(), body.clone())))?;
        let session = match plan {
            Some(a) => {
                let args = json!({"name": name, "cols": a.get("cols"), "rows": a.get("rows")});
                self.dispatch("plan_item", &args)?["session"].take()
            }
            None => Json::Null,
        };
        Ok(json!({"item": to_json(item)?, "session": session}))
    }

    /// Grupo nuevo (`<prefijo><slug>`) con su README, y los items elegidos adentro (también los que
    /// están en otro grupo; los grupos no se anidan). Un item que no se puede mover no corta el resto:
    /// vuelve en `errors`.
    fn create_group(&self, title: &str, members: &Json) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let wf = ws.workflow();
        let title = squash(title);
        if title.is_empty() {
            return Err(ApiError::new("group.titleRequired", &[]));
        }
        let items: IndexMap<String, Item> =
            ws.list_items()?.into_iter().map(|it| (it.name.clone(), it)).collect();
        let picked: Vec<&Item> = members
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .filter_map(|n| items.get(n))
            .filter(|it| it.kind != ItemKind::Group)
            .collect();
        if picked.is_empty() {
            return Err(ApiError::new("group.pickItems", &[]));
        }
        let base = format!("{}{}", wf.folders.group_prefix, slugify(&title, 40, "wi"));
        let path = ws.create_folder(&base)?;
        let name = ws.rel(&path);
        let ids: Vec<String> =
            picked.iter().flat_map(|it| it.ids.iter().map(|id| format!("#{id}"))).collect();
        let body = group_body(wf, &title, &ids);
        ws.write_readme(&name, |_, _| Ok(Change::Parts(Meta::new(), body.clone())))?;
        let (mut moved, mut errors) = (vec![], vec![]);
        for it in picked {
            let res = ws.move_folder(&it.name, &name).map_err(ApiError::from).and_then(|new| {
                self.follow_move(&ws, &it.name, &new)?;
                Ok(new)
            });
            match res {
                Ok(new) => moved.push(new),
                Err(e) => errors.push(json!({"name": it.name, "error": e})),
            }
        }
        Ok(json!({"group": name, "members": moved, "errors": errors}))
    }
}

/// Datos de un item nuevo para su README.
pub(crate) struct NewItem {
    /// Id del tracker (vacío: un item sin id).
    pub id: String,
    pub title: String,
    pub local_type: String,
    pub folder: String,
    pub description: String,
    pub criteria: String,
    /// Tipo y estado en el tracker, y su nombre, para la línea de origen.
    pub tracker_type: String,
    pub tracker_state: String,
    pub source: String,
}

/// Espacios repetidos a uno, sin los de los extremos.
pub(crate) fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Carpeta de un item sin id: `newItem.folderTemplate` con el tipo y el slug del título.
pub(crate) fn folder_without_id(wf: &Workflow, item_type: &str, title: &str) -> String {
    let slug = slugify(title, 40, "wi");
    let tpl = &wf.new_item.folder_template;
    let tpl = if tpl.is_empty() { "{slug}" } else { tpl.as_str() };
    tpl.replace("{type}", item_type).replace("{slug}", &slug).trim_matches('-').to_string()
}

/// README de un item nuevo como la v1: título, línea de origen, descripción y criterios (si hay),
/// la sección de plan con su texto de relleno y la de tareas. Títulos y textos salen del preset.
pub(crate) fn new_item_body(wf: &Workflow, name: &str, d: &NewItem) -> String {
    let rules = &wf.new_item;
    let origin = if d.id.is_empty() {
        rules.origin_no_id.clone()
    } else {
        let kind = if d.tracker_type.is_empty() { &rules.unknown_type } else { &d.tracker_type };
        let mut line = rules.origin.replace("{type}", kind).replace("{id}", &d.id);
        if !d.tracker_state.is_empty() {
            line.push_str(
                &rules.origin_state.replace("{source}", &d.source).replace("{state}", &d.tracker_state),
            );
        }
        line
    };
    let mut body = format!("# {}\n\n", if d.title.is_empty() { name } else { &d.title });
    if !origin.trim().is_empty() {
        body.push_str(&format!("{}\n\n", origin.trim_end()));
    }
    for (heading, text) in [(&wf.sections.description, &d.description), (&wf.sections.criteria, &d.criteria)]
    {
        if !text.is_empty() {
            body.push_str(&format!("## {heading}\n\n{text}\n\n"));
        }
    }
    body.push_str(&format!(
        "## {}\n\n{}\n\n## {}\n\n",
        wf.plan.heading, wf.plan.placeholder, wf.tasks.heading
    ));
    body
}

/// README de un grupo nuevo como la v1: título, línea de origen con los ids, la sección de deploy y la
/// de tareas.
pub(crate) fn group_body(wf: &Workflow, title: &str, ids: &[String]) -> String {
    let g = &wf.group;
    let list = if ids.is_empty() { String::new() } else { g.ids.replace("{list}", &ids.join(", ")) };
    let mut body = format!("# {title}\n\n");
    let origin = g.origin.replace("{ids}", &list);
    if !origin.trim().is_empty() {
        body.push_str(&format!("{}\n\n", origin.trim_end()));
    }
    if !g.deploy_heading.is_empty() {
        body.push_str(&format!("## {}\n\n", g.deploy_heading));
    }
    body.push_str(&format!("## {}\n\n", wf.tasks.heading));
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_copy(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("wb-api-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/fixtures/work");
        fn copy(src: &Path, dest: &Path) {
            std::fs::create_dir_all(dest).unwrap();
            for e in std::fs::read_dir(src).unwrap().flatten() {
                if e.path().is_dir() {
                    copy(&e.path(), &dest.join(e.file_name()));
                } else {
                    std::fs::copy(e.path(), dest.join(e.file_name())).unwrap();
                }
            }
        }
        copy(&src, &base.join("work"));
        (base.join("work"), base)
    }

    #[test]
    fn recents_and_aliases() {
        let (root, base) = fixture_copy("alias");
        let api = Api::new(base.join("cfg"));
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es"}})).unwrap();
        let s = api.dispatch("open_root", &json!({"path": root})).unwrap();
        let open = s["root"].as_str().unwrap().to_string();
        let gone = base.join("gone").display().to_string();
        api.dispatch("restore_recent", &json!({"path": gone, "pos": 5})).unwrap();

        let s = api.dispatch("set_alias", &json!({"path": root, "name": " Mi  proyecto "})).unwrap();
        assert_eq!(s["aliases"][&open], "Mi proyecto");
        let list = api.dispatch("recent_folders", &json!({})).unwrap();
        assert_eq!(list[0]["name"], "Mi proyecto");
        assert_eq!(list[0]["current"], true);
        assert_eq!(list[0]["exists"], true);
        assert_eq!(list[1]["name"], "gone");
        assert_eq!(list[1]["exists"], false);
        assert_eq!(list[1]["alias"], Json::Null);

        let err = api.dispatch("set_alias", &json!({"path": root, "name": "x".repeat(61)})).unwrap_err();
        assert_eq!(err.code, "projects.aliasTooLong");
        let s = api.dispatch("set_alias", &json!({"path": root, "name": ""})).unwrap();
        assert!(s["aliases"].as_object().unwrap().is_empty());

        let s = api.dispatch("forget_recent", &json!({"path": open.to_uppercase()})).unwrap();
        assert_eq!(s["recents"], json!([gone]));
        let s = api.dispatch("restore_recent", &json!({"path": open, "pos": 0})).unwrap();
        assert_eq!(s["recents"][0], open.as_str());
        // Lo guardado sobrevive a otra instancia.
        let again = Api::new(base.join("cfg"));
        assert_eq!(
            again.dispatch("get_settings", &json!({})).unwrap()["recents"].as_array().unwrap().len(),
            2
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn open_root_list_and_edit() {
        let (root, base) = fixture_copy("flow");
        let api = Api::new(base.join("cfg"));
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es"}})).unwrap();
        let err = api.dispatch("list_items", &json!({})).unwrap_err();
        assert_eq!(err.code, "error.noRoot");
        api.dispatch("open_root", &json!({"path": root})).unwrap();
        let items = api.dispatch("list_items", &json!({})).unwrap();
        assert!(items["items"].as_array().unwrap().len() > 5);

        let bad = api
            .dispatch("update_fields", &json!({"name": "wi222222-333333-pagos", "fields": {"estado": "x"}}))
            .unwrap_err();
        assert_eq!(bad.code, "core.field.oneOf");
        assert_eq!(bad.params["key"], "estado");
        let ok = api
            .dispatch(
                "update_fields",
                &json!({"name": "wi222222-333333-pagos", "fields": {"estado": "prod"}}),
            )
            .unwrap();
        assert_eq!(ok["meta"]["estado"], "prod");

        let created = api
            .dispatch("create_item", &json!({"folder": "wi123456-nuevo", "title": "Nuevo", "type": "bug"}))
            .unwrap();
        assert_eq!(created["item"]["meta"]["estado"], "pendiente");
        assert_eq!(created["session"], Json::Null);
        let group = api
            .dispatch("create_group", &json!({"title": "Pagos y más", "members": ["wi123456-nuevo"]}))
            .unwrap();
        assert_eq!(group["group"], "grp-pagos-y-mas");
        assert_eq!(group["members"][0], "grp-pagos-y-mas/wi123456-nuevo");

        assert_eq!(api.dispatch("nope", &json!({})).unwrap_err().code, "error.unknownCommand");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn move_task_between_items_and_inbox() {
        let (root, base) = fixture_copy("move-task");
        let api = Api::new(base.join("cfg"));
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": root})).unwrap();
        let tasks = |name: &str| -> Vec<(String, bool)> {
            let list = api.dispatch("list_items", &json!({})).unwrap();
            let it = list["items"].as_array().unwrap().iter().find(|i| i["name"] == name).unwrap().clone();
            it["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| (t["text"].as_str().unwrap().to_string(), t["done"].as_bool().unwrap()))
                .collect()
        };
        let sso = "wi111111-login-con-sso";
        let pagos = "wi222222-333333-pagos";
        let before = tasks(sso);
        assert_eq!(before[1], ("Crear la tabla de sesiones".to_string(), true));

        // Texto que no coincide: no toca nada.
        let err = api
            .dispatch("move_task", &json!({"src": sso, "index": 1, "expect": "otra", "dst": pagos}))
            .unwrap_err();
        assert_eq!(err.code, "core.task.changed");
        assert_eq!(tasks(sso), before);

        // De un item a otro: conserva el tildado y se va del origen.
        api.dispatch(
            "move_task",
            &json!({"src": sso, "index": 1, "expect": "Crear la tabla de sesiones", "dst": pagos}),
        )
        .unwrap();
        assert_eq!(tasks(sso).len(), before.len() - 1);
        assert_eq!(tasks(pagos).last().unwrap(), &("Crear la tabla de sesiones".to_string(), true));

        // A la bandeja y de vuelta.
        api.dispatch("move_task", &json!({"src": pagos, "index": 0, "expect": "Verificar en QA", "dst": ""}))
            .unwrap();
        let inbox = api.dispatch("inbox", &json!({})).unwrap();
        assert_eq!(inbox[0]["text"], "Verificar en QA");
        assert!(!tasks(pagos).iter().any(|t| t.0 == "Verificar en QA"));
        api.dispatch("move_task", &json!({"src": "", "index": 0, "expect": "Verificar en QA", "dst": sso}))
            .unwrap();
        assert_eq!(api.dispatch("inbox", &json!({})).unwrap().as_array().unwrap().len(), 0);
        assert_eq!(tasks(sso).last().unwrap().0, "Verificar en QA");

        // Mismo origen y destino: no hace nada.
        assert!(
            api.dispatch("move_task", &json!({"src": sso, "index": 0, "expect": "x", "dst": sso}))
                .unwrap()
                .is_null()
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn apply_learnings_opens_a_session_on_the_note_items() {
        let (root, base) = fixture_copy("learn");
        let exe = std::env::current_exe().unwrap();
        let api = Api::new(base.join("cfg"));
        api.set_session_bins(exe.clone(), exe);
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es", "lang": "es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": root})).unwrap();
        let (cmd, args) =
            if cfg!(windows) { ("cmd", "/c echo {session_id}") } else { ("sh", "-c \"echo {session_id}\"") };
        api.dispatch("set_agents", &json!({"default": "custom", "custom": {"command": cmd, "args": args}}))
            .unwrap();

        let loose = api.dispatch("note_create", &json!({"title": "Aprendizajes · nada"})).unwrap();
        let err = api.dispatch("apply_learnings", &json!({"id": loose["id"]})).unwrap_err();
        assert_eq!(err.code, "learn.noItems");

        let note = api
            .dispatch("note_create", &json!({"title": "Aprendizajes · 111111", "items": ["111111", "nope"]}))
            .unwrap();
        let out = api.dispatch("apply_learnings", &json!({"id": note["id"], "which": " 1,  3 "})).unwrap();
        let sess = &out["session"];
        assert_eq!(sess["mode"], "default");
        assert_eq!(sess["name"], "Aplicar aprendizajes · Aprendizajes · 111111");
        // 111111 está en dos carpetas: la primera que lo tiene.
        assert_eq!(sess["items"].as_array().unwrap().len(), 1);
        api.shutdown();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn promote_tool_replaces_with_another_script_and_needs_no_description() {
        let (root, base) = fixture_copy("promote");
        let exe = std::env::current_exe().unwrap();
        let api = Api::new(base.join("cfg"));
        api.set_session_bins(exe.clone(), exe);
        api.dispatch("open_root", &json!({"path": root})).unwrap();
        let first = base.join("count.py");
        std::fs::write(
            &first,
            "print(1)
",
        )
        .unwrap();
        wb_sessions::tools::promote(
            &base.join("cfg"),
            &first,
            "count",
            "Cuenta",
            "",
            json!({"wis": ["1"]}),
            false,
        )
        .unwrap();
        let other = base.join("count2.py");
        std::fs::write(
            &other,
            "print(2)
",
        )
        .unwrap();
        let args = |replace: bool| json!({"path": other, "name": "count", "replace": replace});
        // Sin reemplazar, un script que no es candidato no se promueve.
        assert_eq!(api.dispatch("tool_promote", &args(false)).unwrap_err().code, "tools.error.notCandidate");
        let t = api.dispatch("tool_promote", &args(true)).unwrap();
        assert_eq!(t["file"], "count2.py");
        assert_eq!(t["description"], "");
        assert_eq!(t["origin"]["items"], json!(["1"]));
        // Una tool que no existe no se "reemplaza".
        let nope = json!({"path": other, "name": "nope", "replace": true});
        assert_eq!(api.dispatch("tool_promote", &nope).unwrap_err().code, "tools.error.notCandidate");
        api.shutdown();
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn sessions_plan_and_agents() {
        let (root, base) = fixture_copy("sessions");
        // Este agente no lanza el MCP ni el hook: cualquier archivo existente sirve de binario.
        let exe = std::env::current_exe().unwrap();
        let api = Api::new(base.join("cfg"));
        api.set_session_bins(exe.clone(), exe);
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es", "lang": "es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": root})).unwrap();
        let (cmd, args) =
            if cfg!(windows) { ("cmd", "/c echo {session_id}") } else { ("sh", "-c \"echo {session_id}\"") };
        let info = api
            .dispatch("set_agents", &json!({"default": "custom", "custom": {"command": cmd, "args": args}, "planPrefix": {"claude": "/go ", "nope": "x"}}))
            .unwrap();
        assert_eq!(info["default"], "custom");
        assert_eq!(info["planPrefix"], json!({"claude": "/go "}));
        // Sin etiqueta, la UI muestra la del agente («Comando propio»), no el comando.
        let custom = info["agents"].as_array().unwrap().iter().find(|a| a["id"] == "custom").unwrap();
        assert!(custom.get("label").is_none(), "{custom}");
        assert_eq!(
            api.dispatch("set_agents", &json!({"default": "zzz"})).unwrap_err().code,
            "session.agent.unknown"
        );

        let out = api
            .dispatch("plan_item", &json!({"name": "wi222222-333333-pagos", "cols": 80, "rows": 24}))
            .unwrap();
        assert_eq!(out["session"]["mode"], "plan");
        assert_eq!(out["session"]["plan"], true);
        assert_eq!(out["session"]["name"], "Planear 222222");
        let readme = std::fs::read_to_string(root.join("wi222222-333333-pagos/README.md")).unwrap();
        assert!(readme.contains("estado: plan") && readme.contains("## Plan"), "{readme}");

        let list = api.dispatch("sessions", &json!({})).unwrap();
        assert_eq!(list["sessions"].as_array().unwrap().len(), 1);
        assert!(list["rev"].as_u64().unwrap() > 0);
        let sid = list["sessions"][0]["id"].as_str().unwrap().to_string();

        // Mover la carpeta a un grupo: la sesión la sigue.
        api.dispatch("create_group", &json!({"title": "Pagos", "members": ["wi222222-333333-pagos"]}))
            .unwrap();
        let list = api.dispatch("sessions", &json!({})).unwrap();
        assert_eq!(list["sessions"][0]["items"][0], "grp-pagos/wi222222-333333-pagos");
        assert_eq!(
            api.dispatch("session_prompt", &json!({"id": sid, "kind": "x"})).unwrap_err().code,
            "session.unknownPrompt"
        );
        // Comando propio vacío = borrar su configuración (y deja de ser el de por defecto).
        let info = api
            .dispatch("set_agents", &json!({"custom": {"command": " ", "label": "Mío", "args": "-x"}}))
            .unwrap();
        assert_eq!(info["custom"], json!({"label": "", "command": "", "args": "", "resumeArgs": ""}));
        assert_eq!(info["default"], "claude");
        api.shutdown();
        std::fs::remove_dir_all(&base).ok();
    }

    fn readme(root: &Path, name: &str) -> String {
        std::fs::read_to_string(root.join(name).join("README.md")).unwrap()
    }

    #[test]
    fn new_item_without_id_writes_the_v1_readme() {
        let (root, base) = fixture_copy("new-fix");
        let api = Api::new(base.join("cfg"));
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": root})).unwrap();

        let err = api.dispatch("create_item", &json!({"title": "   "})).unwrap_err();
        assert_eq!(err.code, "newItem.titleRequired");

        // Sin tipo ni carpeta: el tipo «arreglo» del preset y `<tipo>-<slug>` (v1 `_plan_wi_create`).
        let out = api
            .dispatch(
                "create_item",
                &json!({"title": " Ajustar  timeout del job ", "description": " Sube a 60 s.\n", "criteria": ""}),
            )
            .unwrap();
        assert_eq!(out["item"]["name"], "fix-ajustar-timeout-del-job");
        assert_eq!(
            readme(&root, "fix-ajustar-timeout-del-job"),
            "---\nestado: pendiente\ntipo: fix\n---\n# Ajustar timeout del job\n\n\
             > Arreglo del proyecto (sin work item)\n\n## Descripción\n\nSube a 60 s.\n\n\
             ## Plan\n\n*(lo completa la sesión de planificación)*\n\n## Pendientes\n\n"
        );

        // Tipo y carpeta elegidos, sin descripción.
        api.dispatch("create_item", &json!({"title": "Otro", "type": "task", "folder": "task-a-mano"}))
            .unwrap();
        assert_eq!(
            readme(&root, "task-a-mano"),
            "---\nestado: pendiente\ntipo: task\n---\n# Otro\n\n> Arreglo del proyecto (sin work item)\n\n\
             ## Plan\n\n*(lo completa la sesión de planificación)*\n\n## Pendientes\n\n"
        );
        let bad = api.dispatch("create_item", &json!({"title": "x", "type": "nope"})).unwrap_err();
        assert_eq!(bad.code, "error.badArgument");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn tracked_readme_matches_v1() {
        let wf = Workflow::preset("classic-es").unwrap();
        let mut d = NewItem {
            id: "123456".into(),
            title: "Login".into(),
            local_type: "bug".into(),
            folder: "wi123456-login".into(),
            description: "Falla el SSO".into(),
            criteria: "- entra".into(),
            tracker_type: "Bug".into(),
            tracker_state: "Active".into(),
            source: "Azure DevOps".into(),
        };
        assert_eq!(
            new_item_body(&wf, "wi123456-login", &d),
            "# Login\n\n> Bug #123456 · Azure: Active\n\n## Descripción\n\nFalla el SSO\n\n\
             ## Criterios de aceptación\n\n- entra\n\n## Plan\n\n*(lo completa la sesión de planificación)*\n\n\
             ## Pendientes\n\n"
        );
        // A mano (sin datos del tracker): «Work item» y sin estado, título = carpeta.
        d.tracker_type.clear();
        d.tracker_state.clear();
        d.title.clear();
        d.description.clear();
        d.criteria.clear();
        assert_eq!(
            new_item_body(&wf, "wi123456", &d),
            "# wi123456\n\n> Work item #123456\n\n## Plan\n\n*(lo completa la sesión de planificación)*\n\n\
             ## Pendientes\n\n"
        );
        // El preset genérico usa el nombre del tracker.
        let wf = Workflow::preset("default").unwrap();
        d.tracker_state = "Active".into();
        assert!(new_item_body(&wf, "x", &d).contains("> Work item #123456 · Azure DevOps: Active\n"));
    }

    #[test]
    fn create_group_like_v1() {
        let (root, base) = fixture_copy("new-group");
        let api = Api::new(base.join("cfg"));
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": root})).unwrap();

        let err = api.dispatch("create_group", &json!({"title": " ", "members": ["wi111111-login-con-sso"]}));
        assert_eq!(err.unwrap_err().code, "group.titleRequired");
        // Sin items válidos (un grupo no cuenta): no se crea nada.
        let err =
            api.dispatch("create_group", &json!({"title": "Vacío", "members": ["grp-checkout", "nope"]}));
        assert_eq!(err.unwrap_err().code, "group.pickItems");
        assert!(!root.join("grp-vacio").exists());

        // Un item suelto y uno de otro grupo (se reagrupa); cada uno se mueve por su cuenta.
        let out = api
            .dispatch(
                "create_group",
                &json!({"title": "Pagos", "members": ["grp-checkout/wi444444-carrito", "wi222222-333333-pagos"]}),
            )
            .unwrap();
        assert_eq!(out["members"], json!(["grp-pagos/wi444444-carrito", "grp-pagos/wi222222-333333-pagos"]));
        assert_eq!(out["errors"], json!([]));
        assert_eq!(
            readme(&root, "grp-pagos"),
            "# Pagos\n\n> Grupo de WI: #444444, #222222, #333333. Cada WI tiene su carpeta (y su estado) \
             adentro; acá va lo compartido entre ellos.\n\n## Cómo deployar\n\n## Pendientes\n\n"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn create_and_plan_in_one_step() {
        let (root, base) = fixture_copy("new-plan");
        let exe = std::env::current_exe().unwrap();
        let api = Api::new(base.join("cfg"));
        api.set_session_bins(exe.clone(), exe);
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es", "lang": "es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": root})).unwrap();
        let (cmd, args) =
            if cfg!(windows) { ("cmd", "/c echo {session_id}") } else { ("sh", "-c \"echo {session_id}\"") };
        api.dispatch("set_agents", &json!({"default": "custom", "custom": {"command": cmd, "args": args}}))
            .unwrap();

        let out = api
            .dispatch("create_item", &json!({"title": "Arreglar job", "plan": true, "cols": 80, "rows": 24}))
            .unwrap();
        assert_eq!(out["item"]["meta"]["estado"], "plan");
        assert_eq!(out["session"]["mode"], "plan");
        assert_eq!(out["session"]["name"], "Planear fix-arreglar-job");
        // Ya se crea en el estado de plan (como la v1); después solo se suma la sesión.
        let text = readme(&root, "fix-arreglar-job");
        let sid = out["session"]["id"].as_str().unwrap();
        assert_eq!(
            text,
            format!(
                "---\nestado: plan\ntipo: fix\nsesiones: [{sid}]\n---\n# Arreglar job\n\n\
                 > Arreglo del proyecto (sin work item)\n\n## Plan\n\n*(lo completa la sesión de planificación)*\n\n\
                 ## Pendientes\n\n"
            )
        );
        api.shutdown();
        std::fs::remove_dir_all(&base).ok();
    }
}
