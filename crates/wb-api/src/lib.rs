//! API de la app: un único `dispatch(cmd, args)` que usan la ventana (Tauri, comando `api`) y el servidor
//! de desarrollo (`wb-devserver`, `POST /api/<cmd>`). Cada comando es un wrapper fino sobre `wb-core`.
//! Todo se lee en cada llamada: la carpeta de trabajos puede cambiar por fuera (agentes, el usuario).

mod alerts;
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

    /// Ejecuta un comando. Los errores siempre vuelven como `ApiError` (nunca un panic ni texto armado).
    pub fn dispatch(&self, cmd: &str, args: &Json) -> ApiResult<Json> {
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
                let runs = self.runs().of_root(ws.root());
                let runs: serde_json::Map<String, Json> = item
                    .files
                    .iter()
                    .filter_map(|f| runs.get(&f.path).map(|m| (f.path.clone(), json!(m))))
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
            "read_file" => to_json(self.workspace()?.read_file(a.str("path")?)?),
            "read_media" => to_json(self.workspace()?.read_media(a.str("path")?)?),
            "list_versions" => to_json(self.workspace()?.list_versions(a.str("name")?)?),
            "read_version" => to_json(self.workspace()?.read_version(a.str("name")?, a.str("id")?)?),
            "restore_version" => to_json(self.workspace()?.restore_version(a.str("name")?, a.str("id")?)?),
            "create_item" => {
                self.create_item(a.str("folder")?, a.str("title")?, a.opt_str("type").unwrap_or(""))
            }
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
                let marks = self.runs().mark(
                    ws.workflow(),
                    ws.root(),
                    a.str("path")?,
                    a.str("env")?,
                    a.bool("on"),
                    None,
                )?;
                to_json(marks)
            }
            "open_in_editor" => {
                let ws = self.workspace()?;
                let p = ws.safe_path(a.opt_str("path").unwrap_or(""))?;
                if !wb_platform::open_in_editor(&p)? {
                    return Err(ApiError::new("palette.editorNotFound", &[]));
                }
                Ok(Json::Null)
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

    /// Carpeta nueva con su README: estado inicial (el primero "por hacer" del flujo) y tipo.
    fn create_item(&self, folder: &str, title: &str, item_type: &str) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let wf = ws.workflow();
        let initial = wf
            .states
            .iter()
            .find(|s| s.category == StateCategory::Backlog)
            .unwrap_or(&wf.states[0])
            .id
            .clone();
        if !item_type.is_empty() && !wf.types.iter().any(|t| t.id == item_type) {
            return Err(ApiError::new("error.badArgument", &[("name", "type")]));
        }
        let path = ws.create_folder(folder)?;
        let name = ws.rel(&path);
        let (status_key, type_key) = (wf.fields.status.clone(), wf.fields.kind.clone());
        let title = title.trim().to_string();
        let item = ws.write_readme(&name, |_, _| {
            let mut meta = Meta::new();
            meta.insert(status_key.clone(), Value::Text(initial.clone()));
            if !item_type.is_empty() {
                meta.insert(type_key.clone(), Value::Text(item_type.to_string()));
            }
            Ok(Change::Parts(meta, format!("# {}\n", if title.is_empty() { &name } else { &title })))
        })?;
        to_json(item)
    }

    /// Grupo nuevo (`<prefijo><slug>`) con su README, y los items elegidos adentro.
    fn create_group(&self, title: &str, members: &Json) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let base = format!("{}{}", ws.workflow().folders.group_prefix, slugify(title, 40, "group"));
        let path = ws.create_folder(&base)?;
        let name = ws.rel(&path);
        let title = title.trim().to_string();
        ws.write_readme(&name, |meta, _| Ok(Change::Parts(meta, format!("# {title}\n"))))?;
        let mut moved = vec![];
        for m in members.as_array().into_iter().flatten().filter_map(Json::as_str) {
            let new = ws.move_folder(m, &name)?;
            self.follow_move(&ws, m, &new)?;
            moved.push(new);
        }
        Ok(json!({"group": name, "members": moved}))
    }
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
        assert_eq!(created["meta"]["estado"], "pendiente");
        let group = api
            .dispatch("create_group", &json!({"title": "Pagos y más", "members": ["wi123456-nuevo"]}))
            .unwrap();
        assert_eq!(group["group"], "grp-pagos-y-mas");
        assert_eq!(group["members"][0], "grp-pagos-y-mas/wi123456-nuevo");

        assert_eq!(api.dispatch("nope", &json!({})).unwrap_err().code, "error.unknownCommand");
        std::fs::remove_dir_all(&base).unwrap();
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
        api.shutdown();
        std::fs::remove_dir_all(&base).ok();
    }
}
