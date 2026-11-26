//! Plugins en la app: listar, habilitar y correr acciones. Cada paso de una acción lanza el plugin
//! (`wb_plugins::call`) y atiende sus pedidos `host/…` con los permisos de su manifiesto. Lo que devuelve
//! (formulario, resultado, sesión o cerrar) se valida acá antes de llegar a la UI.

use std::collections::BTreeSet;
use std::path::Path;

use indexmap::IndexMap;
use serde_json::{Value as Json, json};
use wb_core::frontmatter::Value;
use wb_core::workspace::{Item, ItemKind};
use wb_core::{Workflow, Workspace};
use wb_plugins::{Manifest, Plugins};
use wb_sessions::registry::{Registry, Session, State as SessionState};

use crate::{Api, ApiError, ApiResult, Args};

/// Tope de los datos propios de un plugin y de un archivo leído con `host/readFile`.
const DATA_MAX: usize = 1024 * 1024;
const FILE_MAX: u64 = 2 * 1024 * 1024;
/// Texto de un campo extra del frontmatter que escribe un plugin.
const EXTRA_MAX: usize = 200;

fn str_list(v: &Json) -> Vec<String> {
    v.as_array().map_or(vec![], |l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect())
}

/// Sesiones no cerradas de la carpeta de trabajos (para `sessions` de cada item).
fn open_sessions(config_dir: &Path, root: &Path) -> Vec<Session> {
    Registry::of_root(config_dir, root)
        .load()
        .into_iter()
        .filter(|s| s.state != SessionState::Closed)
        .collect()
}

/// Un item como lo ve un plugin. `sessions`: cuántas sesiones abiertas (no cerradas) lo incluyen.
fn item_ref(wf: &Workflow, root: &Path, it: &Item, open: &[Session]) -> Json {
    json!({
        "sessions": open.iter().filter(|s| s.items.contains(&it.name)).count(),
        "name": it.name,
        "title": it.title,
        "ids": it.ids,
        "kind": if it.kind == ItemKind::Group { "group" } else { "item" },
        "type": it.item_type,
        "status": it.meta.get(&wf.fields.status).and_then(Value::as_text).unwrap_or(""),
        "folder": root.join(&it.name).display().to_string(),
    })
}

/// Atiende los pedidos de un plugin durante una llamada.
struct ApiHost<'a> {
    api: &'a Api,
    manifest: &'a Manifest,
    ws: Option<Workspace>,
}

impl ApiHost<'_> {
    fn need(&self, permission: &str) -> Result<(), String> {
        if self.manifest.allows(permission) { Ok(()) } else { Err("plugin.denied".into()) }
    }

    fn ws(&self) -> Result<&Workspace, String> {
        self.ws.as_ref().ok_or_else(|| "error.noRoot".to_string())
    }

    fn secret_key(&self, params: &Json) -> Result<String, String> {
        let key =
            params["key"].as_str().filter(|k| !k.is_empty() && k.len() <= 200).ok_or("plugin.badArgument")?;
        Ok(format!("plugin:{}:{key}", self.manifest.id))
    }

    fn open_sessions(&self, ws: &Workspace) -> Vec<Session> {
        open_sessions(&self.api.config_dir, ws.root())
    }

    fn data_path(&self) -> std::path::PathBuf {
        self.api.config_dir.join("plugin-data").join(format!("{}.json", self.manifest.id))
    }

    fn item(&self, name: &str) -> Result<Json, String> {
        let ws = self.ws()?;
        let it = ws.folder_of(name).and_then(|f| ws.build_item(&f)).map_err(|e| e.code.to_string())?;
        let mut out = item_ref(ws.workflow(), ws.root(), &it, &self.open_sessions(ws));
        out["meta"] = serde_json::to_value(&it.meta).unwrap_or(Json::Null);
        out["files"] = Json::Array(
            it.files
                .iter()
                .map(|f| json!({"path": f.path, "name": f.name, "kind": f.kind, "category": f.cat, "size": f.size}))
                .collect(),
        );
        Ok(out)
    }

    fn update_item(&self, params: &Json) -> Result<Json, String> {
        self.need("items.write")?;
        let ws = self.ws()?;
        let wf = ws.workflow();
        let name = params["name"].as_str().ok_or("plugin.badArgument")?;
        let fields = params["fields"].as_object().ok_or("plugin.badArgument")?;
        let editable = wf.fields.editable();
        let mut own: IndexMap<String, Json> = IndexMap::new();
        let mut extra: IndexMap<String, Value> = IndexMap::new();
        for (key, value) in fields {
            if editable.contains(&key.as_str()) {
                own.insert(key.clone(), value.clone());
            } else if self.manifest.fields.contains(key) {
                let text = value.as_str().map(String::from).or_else(|| value.as_i64().map(|n| n.to_string()));
                let text = text.ok_or("plugin.badArgument")?;
                if text.chars().count() > EXTRA_MAX || text.contains(['\n', '\r']) {
                    return Err("plugin.badArgument".into());
                }
                extra.insert(key.clone(), Value::Text(text));
            } else {
                return Err("plugin.denied".into());
            }
        }
        let mut all = wb_core::validate::validate_fields(wf, &own).map_err(|e| e.code.to_string())?;
        all.extend(extra);
        ws.folder_of(name).map_err(|e| e.code.to_string())?;
        let it = ws.update_meta(name, &all).map_err(|e| e.code.to_string())?;
        Ok(item_ref(wf, ws.root(), &it, &self.open_sessions(ws)))
    }

    /// Un archivo dentro de la carpeta de algún item (no el README de otra cosa ni nada fuera).
    fn read_file(&self, params: &Json) -> Result<Json, String> {
        self.need("files.read")?;
        let ws = self.ws()?;
        let rel = params["path"].as_str().ok_or("plugin.badArgument")?;
        let path = ws.safe_path(rel).map_err(|e| e.code.to_string())?;
        let items = ws.list_items().map_err(|e| e.code.to_string())?;
        let inside = items
            .iter()
            .any(|it| path.starts_with(ws.root().join(&it.name)) && path != ws.root().join(&it.name));
        if !inside || !path.is_file() {
            return Err("plugin.badArgument".into());
        }
        let meta = std::fs::metadata(&path).map_err(|_| "plugin.badArgument".to_string())?;
        if meta.len() > FILE_MAX {
            return Err("plugin.fileTooLarge".into());
        }
        let bytes = std::fs::read(&path).map_err(|_| "plugin.badArgument".to_string())?;
        Ok(json!({"text": wb_core::frontmatter::decode(&bytes)}))
    }
}

impl wb_plugins::Host for ApiHost<'_> {
    fn request(&mut self, method: &str, params: &Json) -> Result<Json, String> {
        match method {
            "host/workflow" => {
                let wf = self.api.workflow().map_err(|e| e.code)?;
                serde_json::to_value(wf).map_err(|_| "plugin.badArgument".into())
            }
            "host/items" => {
                let ws = self.ws()?;
                let items = ws.list_items().map_err(|e| e.code.to_string())?;
                let open = self.open_sessions(ws);
                Ok(Json::Array(
                    items.iter().map(|it| item_ref(ws.workflow(), ws.root(), it, &open)).collect(),
                ))
            }
            "host/item" => self.item(params["name"].as_str().ok_or("plugin.badArgument")?),
            "host/updateItem" => self.update_item(params),
            "host/readFile" => self.read_file(params),
            "host/secret/get" => {
                self.need("secrets")?;
                let v =
                    self.api.secrets().get(&self.secret_key(params)?).map_err(|_| "plugin.secretStore")?;
                Ok(json!({"value": v}))
            }
            "host/secret/set" => {
                self.need("secrets")?;
                let value = params["value"].as_str().ok_or("plugin.badArgument")?;
                self.api.secrets().set(&self.secret_key(params)?, value).map_err(|_| "plugin.secretStore")?;
                Ok(json!({}))
            }
            "host/secret/delete" => {
                self.need("secrets")?;
                self.api.secrets().delete(&self.secret_key(params)?).map_err(|_| "plugin.secretStore")?;
                Ok(json!({}))
            }
            "host/data/get" => Ok(std::fs::read_to_string(self.data_path())
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_else(|| json!({}))),
            "host/data/set" => {
                let text = serde_json::to_string(&params["data"]).map_err(|_| "plugin.badArgument")?;
                if text.len() > DATA_MAX {
                    return Err("plugin.dataTooLarge".into());
                }
                let path = self.data_path();
                let write = || -> std::io::Result<()> {
                    std::fs::create_dir_all(path.parent().expect("parent"))?;
                    let tmp = path.with_extension("json.tmp");
                    std::fs::write(&tmp, &text)?;
                    std::fs::rename(&tmp, &path)
                };
                write().map_err(|_| "plugin.dataWrite")?;
                Ok(json!({}))
            }
            "host/tracker" => {
                self.need("tracker")?;
                self.api.tracker_info().map_err(|e| e.code)
            }
            // El token de escritura del tracker, compartido con «Cambiar estado en el tracker».
            "host/tracker/writeToken/get"
            | "host/tracker/writeToken/set"
            | "host/tracker/writeToken/delete" => {
                self.need("tracker.write")?;
                let op = method.rsplit('/').next().unwrap_or_default();
                if op == "set" && !params["value"].as_str().is_some_and(|v| !v.trim().is_empty()) {
                    return Err("plugin.badArgument".into());
                }
                self.api.tracker_write_token(op, params["value"].as_str()).map_err(|e| e.code)
            }
            _ => Err("plugin.unknownMethod".into()),
        }
    }
}

impl Api {
    pub(crate) fn plugin_command(&self, cmd: &str, a: &Args) -> Option<ApiResult<Json>> {
        let out = match cmd {
            "plugins" => self.plugins_list(),
            "plugin_enable" => a.str("id").and_then(|id| self.plugin_enable(id, a.bool("enabled"))),
            "plugins_open_folder" => self.plugins_open_folder(),
            "plugin_action" => self.plugin_action(a),
            // Los enlaces de un resultado (solo https: lo vuelve a chequear `open_url`).
            "plugin_open_url" => a.str("url").and_then(|u| {
                if !u.starts_with("https://") {
                    return Err(ApiError::new("error.badArgument", &[("name", "url")]));
                }
                wb_platform::open_url(u).map_err(wb_core::error::CoreError::from)?;
                Ok(Json::Null)
            }),
            _ => return None,
        };
        Some(out)
    }

    fn lang(&self) -> &'static str {
        wb_i18n::resolve_lang(self.settings().lang.as_deref().unwrap_or(wb_i18n::DEFAULT_LANG))
    }

    fn plugins_list(&self) -> ApiResult<Json> {
        let lang = self.lang();
        let all = Plugins::load(&self.config_dir);
        let preset_errors = all.preset_errors();
        let list: Vec<Json> = all
            .list
            .iter()
            .map(|p| {
                let Some(m) = &p.manifest else {
                    return json!({"folder": p.folder, "error": p.error, "enabled": false});
                };
                let errors: Vec<&String> =
                    preset_errors.iter().filter(|(id, _)| id == &m.id).map(|(_, e)| e).collect();
                json!({
                    "id": m.id,
                    "folder": p.folder,
                    "name": m.name.get(lang),
                    "description": m.description.as_ref().map(|d| d.get(lang)),
                    "version": m.version,
                    "enabled": p.enabled,
                    "permissions": m.permissions,
                    "fields": m.fields,
                    "presets": m.presets,
                    "presetErrors": errors,
                    "command": p.argv().err(),
                    "actions": m.actions.iter().map(|x| json!({
                        "id": x.id,
                        "label": x.label.get(lang),
                        "hint": x.hint.as_ref().map(|h| h.get(lang)),
                        "icon": x.icon,
                        "placements": x.placements,
                        "when": x.when,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({"dir": all.dir.display().to_string(), "plugins": list}))
    }

    fn plugin_enable(&self, id: &str, on: bool) -> ApiResult<Json> {
        let all = Plugins::load(&self.config_dir);
        if !all.list.iter().any(|p| p.manifest.as_ref().is_some_and(|m| m.id == id)) {
            return Err(ApiError::new("plugin.notFound", &[("id", id)]));
        }
        // Deshabilitar el plugin del preset en uso dejaría la app sin flujo.
        if !on {
            let current = self.settings().workflow;
            if all.get(id).is_some_and(|_| all.presets().iter().any(|(p, w)| p == id && w.id == current)) {
                return Err(ApiError::new("plugin.presetInUse", &[("preset", &current)]));
            }
        }
        Plugins::set_enabled(&self.config_dir, id, on)
            .map_err(|e| ApiError::new("plugin.save", &[("detail", &e.to_string())]))?;
        self.plugins_list()
    }

    fn plugins_open_folder(&self) -> ApiResult<Json> {
        let dir = Plugins::dir_in(&self.config_dir);
        std::fs::create_dir_all(&dir).map_err(wb_core::error::CoreError::from)?;
        wb_platform::open_path(&dir).map_err(wb_core::error::CoreError::from)?;
        Ok(Json::Null)
    }

    /// Un paso de una acción: `action/open` sin `button`, `action/submit` con.
    fn plugin_action(&self, a: &Args) -> ApiResult<Json> {
        let id = a.str("plugin")?;
        let action_id = a.str("action")?;
        let all = Plugins::load(&self.config_dir);
        let (plugin, manifest) =
            all.get(id).ok_or_else(|| ApiError::new("plugin.notFound", &[("id", id)]))?;
        let action = manifest
            .action(action_id)
            .ok_or_else(|| ApiError::new("plugin.noAction", &[("id", action_id)]))?;
        let ws = self.workspace().ok();
        let names = str_list(a.get("items"));
        let mut items = vec![];
        if !names.is_empty() {
            let ws = ws.as_ref().ok_or_else(|| ApiError::new("error.noRoot", &[]))?;
            let wf = ws.workflow();
            let open = open_sessions(&self.config_dir, ws.root());
            for n in &names {
                let it = ws.build_item(&ws.folder_of(n)?)?;
                let r = item_ref(wf, ws.root(), &it, &open);
                if !wb_plugins::matches(action.when.as_ref(), wf, &r) {
                    return Err(ApiError::new("plugin.notForItem", &[("item", n)]));
                }
                items.push(r);
            }
        }
        let lang = self.lang();
        let root = ws.as_ref().map(|w| w.root().display().to_string());
        let placement = a.opt_str("placement").filter(|p| wb_plugins::PLACEMENTS.contains(p));
        if placement.is_some_and(|p| !action.placements.iter().any(|x| x == p)) {
            return Err(ApiError::new("plugin.noAction", &[("id", action_id)]));
        }
        let context = json!({"lang": lang, "root": root, "items": items, "placement": placement});
        let (method, params) = match a.opt_str("button") {
            None => ("action/open", json!({"action": action_id, "context": context})),
            Some(button) => (
                "action/submit",
                json!({"action": action_id, "context": context, "button": button, "values": a.get("values"), "state": a.get("state")}),
            ),
        };
        let mut host = ApiHost { api: self, manifest, ws };
        let mut stderr = String::new();
        let out = wb_plugins::call(plugin, lang, method, params, &mut host, &mut stderr);
        if !stderr.trim().is_empty() {
            log_stderr(&self.config_dir, &manifest.id, &stderr);
        }
        let step = out.map_err(|e| {
            ApiError::new(e.code, &[("plugin", &manifest.name.get(lang)), ("detail", &e.detail)])
        })?;
        self.plugin_step(step, a)
    }

    /// Valida el paso que devolvió el plugin; una sesión se abre acá.
    fn plugin_step(&self, step: Json, a: &Args) -> ApiResult<Json> {
        let bad = |what: &str| ApiError::new("plugin.error.badStep", &[("detail", what)]);
        match step["type"].as_str() {
            Some("form") => {
                let fields = step["fields"].as_array().ok_or_else(|| bad("fields"))?;
                let kinds = ["text", "textarea", "number", "password", "select", "checkbox", "info"];
                let mut ids = BTreeSet::new();
                for f in fields {
                    let kind = f["type"].as_str().unwrap_or("");
                    if !kinds.contains(&kind) {
                        return Err(bad(&format!("field type {kind:?}")));
                    }
                    if kind != "info"
                        && !f["id"].as_str().is_some_and(|i| !i.is_empty() && ids.insert(i.to_string()))
                    {
                        return Err(bad("field id"));
                    }
                }
                Ok(step)
            }
            Some("result") => {
                let mut step = step;
                if let Some(links) = step["links"].as_array_mut() {
                    links.retain(|l| l["url"].as_str().is_some_and(|u| u.starts_with("https://")));
                }
                Ok(step)
            }
            Some("close") => Ok(step),
            Some("session") => {
                let ws = self.workspace()?;
                let items = str_list(&step["items"]);
                if items.is_empty() {
                    return Err(bad("session items"));
                }
                let mode = step["mode"].as_str().unwrap_or("default");
                let n = |k: &str, d: u16| a.get(k).as_u64().map_or(d, |v| v.clamp(1, 1000) as u16);
                let agent = self.settings().agents.default_agent().to_string();
                let sess = self.session_manager()?.create(
                    &ws,
                    &items,
                    step["name"].as_str().unwrap_or(""),
                    mode,
                    &agent,
                    false,
                    step["model"].as_str(),
                    step["prompt"].as_str(),
                    (n("cols", 120), n("rows", 30)),
                )?;
                Ok(json!({"type": "session", "session": sess, "message": step["message"]}))
            }
            _ => Err(bad("type")),
        }
    }
}

/// Lo que el plugin escribió en stderr, al final de `<config>/plugins.log` (recortado a 1 MB).
fn log_stderr(config_dir: &Path, id: &str, text: &str) {
    use std::io::Write;
    let path = config_dir.join("plugins.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = std::fs::rename(&path, config_dir.join("plugins.log.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "--- {now} {id}\n{}", text.trim_end());
    }
}
