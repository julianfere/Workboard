//! Comandos de trackers (Azure DevOps o el de un plugin, ver `tracker_plugin.rs`): conexión por carpeta raíz, mapeo de estados, campos de fecha
//! pactada y horas, sincronización con previsualización, asignados del período, items nuevos con los
//! datos del tracker y el cambio de estado en el tracker.
//!
//! - Lectura salvo `tracker_push_states`, que escribe en el tracker con la credencial de escritura (aparte
//!   de la de lectura) y solo a pedido del usuario.
//! - Las credenciales van al almacén del sistema (`wb_platform::secrets`), nunca a archivos, logs ni
//!   errores. En la carpeta de config: `trackers.json` (conexiones por raíz y, por proyecto, estados,
//!   mapeo y campos) y `tracker_cache.json` (lo último leído, por raíz; lo lee también el MCP).
//! - En el README solo se escriben el estado y el tipo (al aplicar el sync), o la carpeta nueva de un item.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{Datelike, NaiveDate};
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use wb_core::frontmatter::Value;
use wb_core::records::RunLog;
use wb_core::text::slugify;
use wb_core::validate::validate_fields;
use wb_core::workspace::Item;
use wb_core::{StateCategory, Workflow, Workspace};
use wb_platform::secrets::{OsSecrets, SecretStore};
use wb_tracker::{
    Connection, FieldInfo, FieldRequest, Tracker, TrackerError, TrackerState, Transport, UreqTransport,
    WorkItem, azure,
};

use crate::tracker_plugin::{Kind, PluginClient, plugin_missing};
use crate::{Api, ApiError, ApiResult, Args, NewItem, squash, to_json};

pub const TRACKERS_FILE: &str = "trackers.json";
/// El mismo archivo que lee el MCP (`wb_mcp::TRACKER_CACHE`).
pub const CACHE_FILE: &str = "tracker_cache.json";

/// Dependencias de los trackers: el almacén de secretos y el transporte HTTP (los tests usan otros).
pub(crate) struct TrackerDeps {
    secrets: Mutex<Option<Arc<dyn SecretStore>>>,
    http: Mutex<Option<Arc<dyn Transport>>>,
    /// Ordena las lecturas y escrituras de `trackers.json` y de la caché.
    lock: Mutex<()>,
}

impl Default for TrackerDeps {
    fn default() -> Self {
        Self { secrets: Mutex::new(None), http: Mutex::new(None), lock: Mutex::new(()) }
    }
}

impl From<TrackerError> for ApiError {
    fn from(e: TrackerError) -> Self {
        let params: Vec<(&str, &str)> = e.params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        ApiError::new(e.code, &params)
    }
}

fn store_error(_: std::io::Error) -> ApiError {
    ApiError::new("tracker.secretStore", &[])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FieldRef {
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    pub(crate) name: String,
}

impl From<&FieldInfo> for FieldRef {
    fn from(f: &FieldInfo) -> Self {
        Self { reference: f.reference.clone(), name: f.name.clone() }
    }
}

/// Lo que se sabe de un proyecto del tracker (compartido por las raíces conectadas a él).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ProjectCfg {
    pub(crate) states: Vec<TrackerState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fetched_at: Option<f64>,
    /// Estado del tracker → id de estado del flujo.
    pub(crate) map: BTreeMap<String, String>,
    pub(crate) map_saved: bool,
    /// `None` = todavía no se leyeron los campos.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) date_fields: Option<Vec<FieldRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) num_fields: Option<Vec<FieldRef>>,
    pub(crate) has_priority: bool,
    pub(crate) has_effort: bool,
    /// `None` = sin elegir (se adivina); `""` = el usuario eligió ninguno.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) due_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hours_field: Option<String>,
}

impl ProjectCfg {
    fn request(&self) -> FieldRequest {
        FieldRequest {
            priority: self.has_priority,
            due: self.due_field.clone().filter(|f| !f.is_empty()),
            hours: self.hours_field.clone().filter(|f| !f.is_empty()),
            effort: self.has_effort,
            ..Default::default()
        }
    }

    /// Campos de fecha y numéricos, prioridad y trabajo completado. Si no hay fecha pactada elegida, se
    /// autoelige; una elegida que ya no existe se olvida.
    fn set_fields(&mut self, c: azure::Catalog) {
        self.has_priority = c.has_priority;
        self.has_effort = c.has_effort;
        let dates: Vec<FieldRef> = c.date_fields.iter().map(FieldRef::from).collect();
        let nums: Vec<FieldRef> = c.num_fields.iter().map(FieldRef::from).collect();
        let known = |list: &[FieldRef], f: &str| f.is_empty() || list.iter().any(|x| x.reference == f);
        if self.due_field.as_deref().is_some_and(|f| !known(&dates, f)) {
            self.due_field = None;
        }
        if self.hours_field.as_deref().is_some_and(|f| !known(&nums, f)) {
            self.hours_field = None;
        }
        if self.due_field.is_none() {
            self.due_field = c.due_guess;
        }
        if self.hours_field.is_none() && c.hours_guess.is_some() {
            self.hours_field = c.hours_guess;
        }
        self.date_fields = Some(dates);
        self.num_fields = Some(nums);
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct TrackerFile {
    /// Por raíz (`RunLog::root_key`).
    pub(crate) connections: BTreeMap<String, Connection>,
    /// Por `tipo|org|proyecto` en minúsculas.
    pub(crate) projects: BTreeMap<String, ProjectCfg>,
}

/// Lo último leído del tracker para una raíz.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CacheEntry {
    source: String,
    kind: String,
    org: String,
    project: String,
    /// `None` = nunca sincronizada (solo items importados).
    synced_at: Option<f64>,
    requested: Vec<String>,
    items: BTreeMap<String, WorkItem>,
}

pub(crate) fn project_key(c: &Connection) -> String {
    format!("{}|{}|{}", c.kind, c.org, c.project).to_lowercase()
}

pub(crate) fn secret_key(c: &Connection, write: bool) -> String {
    format!("{}:{}:{}", c.kind, if write { "write" } else { "read" }, c.org.to_lowercase())
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> ApiResult<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value).map_err(ApiError::generic)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn str_list(v: &Json) -> Vec<String> {
    v.as_array()
        .map_or(vec![], |l| l.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).collect())
}

/// Id de work item como lo escribe el usuario (`#123` → `123`).
fn clean_id(id: &str) -> String {
    id.trim().trim_start_matches('#').trim().to_string()
}

/// Tipo del flujo para un tipo del tracker: por id, nombre o alias (sin mayúsculas), o "".
pub fn local_type(wf: &Workflow, tracker_type: &str) -> String {
    let t = tracker_type.trim().to_lowercase();
    if t.is_empty() {
        return String::new();
    }
    wf.types
        .iter()
        .find(|c| {
            c.id.to_lowercase() == t
                || c.labels.values().any(|l| l.to_lowercase() == t)
                || c.aliases.iter().any(|a| a.to_lowercase() == t)
        })
        .map(|c| c.id.clone())
        .unwrap_or_default()
}

/// Estado del flujo que se sugiere para cada estado del tracker: el primero de su categoría.
fn suggested_map(wf: &Workflow, states: &[TrackerState]) -> BTreeMap<String, String> {
    states
        .iter()
        .filter_map(|s| {
            let cat = s.suggested?;
            let local = wf.states.iter().find(|x| x.category == cat)?;
            Some((s.name.clone(), local.id.clone()))
        })
        .collect()
}

/// Avance de un estado del flujo: categoría y, dentro de ella, su escalón de avance (`progress` del preset
/// o el orden; estados con el mismo escalón empatan, como diag y en-curso en la v1). Cancelados, sin
/// categoría y desconocidos quedan fuera de la escala (nunca "retroceden").
fn rank(wf: &Workflow, state: &str) -> Option<(usize, i64)> {
    const ORDER: [StateCategory; 5] = [
        StateCategory::Backlog,
        StateCategory::Planned,
        StateCategory::Active,
        StateCategory::Review,
        StateCategory::Done,
    ];
    let category = wf.state(state)?.category;
    let cat = ORDER.iter().position(|c| *c == category)?;
    Some((cat, wf.progress(state)?))
}

#[cfg(test)]
mod rank_tests {
    use super::*;

    #[test]
    fn diag_ties_with_in_progress() {
        let wf = Workflow::preset("classic-es").unwrap();
        assert_eq!(rank(&wf, "diag"), rank(&wf, "en-curso"));
        assert!(rank(&wf, "codeado") < rank(&wf, "qa"));
        assert!(rank(&wf, "plan") < rank(&wf, "diag"));
        assert_eq!(rank(&wf, "descartado"), None);
        assert_eq!(rank(&wf, "nope"), None);
    }
}

/// Primer día de un período `AAAA-MM` y el primero del siguiente.
fn period_bounds(wf: &Workflow, period: &str) -> Option<(NaiveDate, NaiveDate)> {
    let (y, m) = period.split_once('-')?;
    if y.len() != 4 || m.len() != 2 {
        return None;
    }
    let (y, m): (i32, u32) = (y.parse().ok()?, m.parse().ok()?);
    let day = u32::from(wf.period.start_day).clamp(1, 28);
    let start = NaiveDate::from_ymd_opt(y, m, day)?;
    let end = if m == 12 {
        NaiveDate::from_ymd_opt(y + 1, 1, day)?
    } else {
        NaiveDate::from_ymd_opt(y, m + 1, day)?
    };
    Some((start, end))
}

/// Previsualización del sync: una fila por carpeta con el estado que propone el tracker (por el mapeo)
/// y/o el tipo que le falta. Se tildan completar estados vacíos, avanzar y completar el tipo; retroceder
/// queda destildado. Con varios work items en la carpeta manda el menos avanzado.
fn preview(
    wf: &Workflow,
    items: &[Item],
    cache: &CacheEntry,
    map: &BTreeMap<String, String>,
    only: Option<&BTreeSet<String>>,
    include_differ: bool,
    git: &HashMap<String, (String, Vec<String>)>,
) -> Json {
    let status_key = &wf.fields.status;
    let known = |s: &str| wf.states.iter().any(|x| x.id == s);
    let mut rows = vec![];
    let mut unmapped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut not_found = vec![];
    let (mut equal, mut differ) = (0, 0);
    for it in items {
        if it.ids.is_empty() || only.is_some_and(|o| !o.contains(&it.name)) {
            continue;
        }
        let tracked: Vec<(&String, &WorkItem)> =
            it.ids.iter().filter_map(|id| cache.items.get(id).map(|w| (id, w))).collect();
        if tracked.is_empty() {
            if it.ids.iter().any(|id| cache.requested.contains(id)) {
                not_found.push(it.name.clone());
            }
            continue;
        }
        let cur = it.meta.get(status_key).and_then(Value::as_text).unwrap_or("").to_string();
        let mut pick: Option<(&String, &WorkItem, String)> = None;
        for (id, w) in &tracked {
            match map.get(&w.state).filter(|s| known(s)) {
                None => unmapped.entry(w.state.clone()).or_default().push(it.name.clone()),
                Some(st) => {
                    let r = rank(wf, st).unwrap_or((99, 0));
                    if pick.as_ref().is_none_or(|(_, _, p)| r < rank(wf, p).unwrap_or((99, 0))) {
                        pick = Some((id, w, st.clone()));
                    }
                }
            }
        }
        let (id, w) = pick.as_ref().map_or(tracked[0], |(id, w, _)| (*id, *w));
        let proposed = pick.as_ref().map(|(_, _, s)| s.clone()).unwrap_or_default();
        let mut status = String::new();
        let mut kind = "";
        if !proposed.is_empty() && proposed == cur {
            equal += 1;
        } else if !proposed.is_empty() {
            status = proposed.clone();
            let back = matches!((rank(wf, &cur), rank(wf, &status)), (Some(c), Some(n)) if n < c);
            kind = if cur.is_empty() {
                "fill"
            } else if back {
                "back"
            } else {
                "differ"
            };
            if kind != "fill" {
                differ += 1;
                if !include_differ {
                    status.clear();
                    kind = "";
                }
            }
        }
        let item_type = if it.item_type.is_empty() { local_type(wf, &w.kind) } else { String::new() };
        if !status.is_empty() || !item_type.is_empty() {
            let kind = if kind.is_empty() { "type" } else { kind };
            rows.push(json!({
                "key": it.name,
                "name": it.name,
                "ids": it.ids,
                "tracker": {"id": id, "state": w.state, "title": w.title, "type": w.kind},
                "current": cur,
                "status": status,
                "type": item_type,
                "kind": kind,
                "on": kind != "back",
            }));
        }
        // Según git: el trabajo ya está en un ambiente en todos sus repos y eso es más que lo actual y lo
        // propuesto. Va destildada y al final (si se tildan las dos, gana esta).
        if let Some((env, repos)) = git.get(&it.name) {
            let st = &wf.git.env_states[env];
            let floor = [rank(wf, &cur), rank(wf, &status)].into_iter().flatten().max();
            let off_scale = !cur.is_empty() && rank(wf, &cur).is_none();
            if !off_scale && rank(wf, st).is_some_and(|r| floor.is_none_or(|f| r > f)) {
                rows.push(json!({
                    "key": format!("{}|git", it.name),
                    "name": it.name,
                    "ids": it.ids,
                    "tracker": {"id": id, "state": w.state, "title": w.title, "type": w.kind},
                    "current": cur,
                    "status": st,
                    "type": "",
                    "kind": "git",
                    "why": {"env": env, "repos": repos},
                    "on": false,
                }));
            }
        }
    }
    let order = |k: &str| ["fill", "type", "differ", "back", "git"].iter().position(|x| *x == k).unwrap_or(9);
    rows.sort_by(|a, b| {
        let key = |r: &Json| {
            (order(r["kind"].as_str().unwrap_or("")), r["name"].as_str().unwrap_or("").to_string())
        };
        key(a).cmp(&key(b))
    });
    let fill = rows.iter().filter(|r| r["kind"] == "fill").count();
    let types = rows.iter().filter(|r| r["type"] != "").count();
    let mut unmapped: Vec<(String, Vec<String>)> = unmapped.into_iter().collect();
    unmapped.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    json!({
        "rows": rows,
        "unmapped": unmapped.into_iter().map(|(state, items)| json!({"state": state, "items": items})).collect::<Vec<_>>(),
        "notFound": not_found,
        "equal": equal,
        "differ": differ,
        "fill": fill,
        "types": types,
        "read": cache.requested.iter().filter(|id| cache.items.contains_key(*id)).count(),
        "requested": cache.requested.len(),
        "syncedAt": cache.synced_at,
    })
}

/// Contexto de un comando: la carpeta abierta, su conexión y la config de su proyecto.
struct Ctx {
    ws: Workspace,
    root_key: String,
    file: TrackerFile,
    conn: Option<Connection>,
    /// De qué es la conexión (sin conexión, Azure).
    kind: Kind,
}

impl Ctx {
    fn conn(&self) -> ApiResult<&Connection> {
        self.conn.as_ref().ok_or_else(|| ApiError::new("tracker.notConnected", &[("source", azure::SOURCE)]))
    }

    fn project(&self) -> ProjectCfg {
        self.conn.as_ref().and_then(|c| self.file.projects.get(&project_key(c))).cloned().unwrap_or_default()
    }

    fn project_mut(&mut self) -> ApiResult<&mut ProjectCfg> {
        let key = project_key(self.conn()?);
        Ok(self.file.projects.entry(key).or_default())
    }
}

impl Api {
    fn tracker_lock(&self) -> MutexGuard<'_, ()> {
        self.tracker.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn secrets(&self) -> Arc<dyn SecretStore> {
        self.tracker
            .secrets
            .lock()
            .expect("secrets")
            .get_or_insert_with(|| Arc::new(OsSecrets::from_env()))
            .clone()
    }

    fn http(&self) -> Arc<dyn Transport> {
        self.tracker
            .http
            .lock()
            .expect("http")
            .get_or_insert_with(|| Arc::new(UreqTransport::default()))
            .clone()
    }

    /// Usa otro almacén de secretos y otro transporte (tests, desarrollo sin red).
    pub fn set_tracker_backend(&self, secrets: Arc<dyn SecretStore>, http: Arc<dyn Transport>) {
        *self.tracker.secrets.lock().expect("secrets") = Some(secrets);
        *self.tracker.http.lock().expect("http") = Some(http);
    }

    fn trackers_path(&self) -> PathBuf {
        self.config_dir.join(TRACKERS_FILE)
    }

    fn cache_path(&self) -> PathBuf {
        self.config_dir.join(CACHE_FILE)
    }

    fn tracker_ctx(&self) -> ApiResult<Ctx> {
        let ws = self.workspace()?;
        let root_key = RunLog::root_key(ws.root());
        let file: TrackerFile = read_json(&self.trackers_path());
        let conn = file.connections.get(&root_key).cloned();
        let kind = conn.as_ref().map_or(Kind::Azure, |c| self.tracker_kind(&c.kind));
        Ok(Ctx { ws, root_key, file, conn, kind })
    }

    fn save_trackers(&self, ctx: &Ctx) -> ApiResult<()> {
        write_json(&self.trackers_path(), &ctx.file)
    }

    fn read_cache(&self, root_key: &str) -> CacheEntry {
        let all: BTreeMap<String, Json> = read_json(&self.cache_path());
        all.get(root_key).and_then(|e| serde_json::from_value(e.clone()).ok()).unwrap_or_default()
    }

    fn write_cache(&self, root_key: &str, entry: Option<&CacheEntry>) -> ApiResult<()> {
        let mut all: BTreeMap<String, Json> = read_json(&self.cache_path());
        match entry {
            Some(e) => {
                all.insert(root_key.to_string(), serde_json::to_value(e).map_err(ApiError::generic)?);
            }
            None => {
                if all.remove(root_key).is_none() {
                    return Ok(());
                }
            }
        }
        write_json(&self.cache_path(), &all)
    }

    fn has_secret(&self, conn: &Connection, write: bool) -> bool {
        self.secrets().get(&secret_key(conn, write)).ok().flatten().is_some()
    }

    /// Nombre para mostrar del tracker de la carpeta.
    fn source(&self, ctx: &Ctx) -> String {
        ctx.kind.source(self.lang())
    }

    /// Cliente con la credencial de lectura (o la de escritura, si el tracker tiene una aparte).
    fn tracker_client<'a>(&'a self, ctx: &Ctx, write: bool) -> ApiResult<Box<dyn Tracker + 'a>> {
        let conn = ctx.conn()?;
        if let Kind::Missing(k) = &ctx.kind {
            return Err(plugin_missing(k).into());
        }
        let write = write && ctx.kind.write_secret();
        let secret = self.secrets().get(&secret_key(conn, write)).map_err(store_error)?.ok_or_else(|| {
            ApiError::new(
                if write { "tracker.noWriteSecret" } else { "tracker.noSecret" },
                &[("source", &self.source(ctx))],
            )
        })?;
        match &ctx.kind {
            Kind::Plugin(p) => Ok(Box::new(PluginClient {
                api: self,
                kind: (**p).clone(),
                conn: conn.clone(),
                secret,
                lang: self.lang(),
            })),
            _ => Ok(conn.client(&secret, self.http())?),
        }
    }

    fn tracker_config(&self, ctx: &Ctx) -> ApiResult<Json> {
        let wf = ctx.ws.workflow();
        let proc = ctx.project();
        let cache = self.read_cache(&ctx.root_key);
        let mut tracker_types: BTreeSet<&str> =
            proc.states.iter().flat_map(|s| s.types.iter().map(String::as_str)).collect();
        tracker_types.extend(cache.items.values().map(|w| w.kind.as_str()).filter(|k| !k.is_empty()));
        let local_types: BTreeMap<&str, String> = tracker_types
            .into_iter()
            .map(|t| (t, local_type(wf, t)))
            .filter(|(_, l)| !l.is_empty())
            .collect();
        let conn = ctx.conn.clone().unwrap_or_default();
        let connected = ctx.conn.is_some();
        Ok(json!({
            "kinds": self.tracker_kinds()?,
            "connected": connected,
            "kind": if connected { conn.kind.as_str() } else { azure::KIND },
            "source": self.source(ctx),
            // El plugin de la conexión está deshabilitado o no está: todo responde `tracker.pluginMissing`.
            "kindMissing": connected && matches!(ctx.kind, Kind::Missing(_)),
            "writeSecret": ctx.kind.write_secret(),
            "org": conn.org,
            "project": conn.project,
            "hasSecret": connected && self.has_secret(&conn, false),
            "hasWriteSecret": connected && self.has_secret(&conn, ctx.kind.write_secret()),
            "states": proc.states,
            "statesFetchedAt": proc.fetched_at,
            "map": proc.map,
            "mapSaved": proc.map_saved,
            "suggested": suggested_map(wf, &proc.states),
            "fieldsFetched": proc.date_fields.is_some(),
            "dateFields": proc.date_fields.clone().unwrap_or_default(),
            "numFields": proc.num_fields.clone().unwrap_or_default(),
            "hasPriority": proc.has_priority,
            "hasEffort": proc.has_effort,
            "dueField": proc.due_field.clone().unwrap_or_default(),
            "hoursField": proc.hours_field.clone().unwrap_or_default(),
            "localTypes": local_types,
            "cache": if cache.source.is_empty() { Json::Null } else { to_json(&cache)? },
        }))
    }

    /// La conexión de la carpeta abierta, sin secretos (para los plugins).
    pub(crate) fn tracker_info(&self) -> ApiResult<Json> {
        let _guard = self.tracker_lock();
        let ctx = self.tracker_ctx()?;
        Ok(ctx.conn.as_ref().map_or(
            Json::Null,
            |c| json!({"kind": c.kind, "source": self.source(&ctx), "org": c.org, "project": c.project}),
        ))
    }

    /// El token de escritura de la organización de la carpeta abierta, el mismo que usa «Cambiar estado en
    /// el tracker» (para los plugins con el permiso `tracker.write`). `op`: `get` → `{value}` (o `null`),
    /// `set` (`value` no vacío) o `delete`.
    pub(crate) fn tracker_write_token(&self, op: &str, value: Option<&str>) -> ApiResult<Json> {
        let _guard = self.tracker_lock();
        let ctx = self.tracker_ctx()?;
        let key = secret_key(ctx.conn()?, true);
        let secrets = self.secrets();
        match op {
            "get" => Ok(json!({"value": secrets.get(&key).map_err(store_error)?})),
            "set" => {
                let value = value
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| ApiError::new("error.badArgument", &[("name", "value")]))?;
                secrets.set(&key, value).map_err(store_error)?;
                Ok(json!({}))
            }
            "delete" => {
                secrets.delete(&key).map_err(store_error)?;
                Ok(json!({}))
            }
            _ => Err(ApiError::new("error.badArgument", &[("name", "op")])),
        }
    }

    pub(crate) fn tracker_command(&self, cmd: &str, a: &Args) -> Option<ApiResult<Json>> {
        if !cmd.starts_with("tracker_") {
            return None;
        }
        let _guard = self.tracker_lock();
        let ctx = match self.tracker_ctx() {
            Ok(c) => c,
            Err(e) => return Some(Err(e)),
        };
        Some(match cmd {
            "tracker_config" => self.tracker_config(&ctx),
            "tracker_connect" => self.tracker_connect(ctx, a),
            "tracker_fetch" => self.tracker_fetch(ctx),
            "tracker_set_map" => self.tracker_set_map(ctx, a.get("map")),
            "tracker_set_field" => {
                self.tracker_set_field(ctx, a.str("which").unwrap_or(""), a.opt_str("ref"))
            }
            "tracker_clear_secret" => self.tracker_clear_secret(&ctx, a.bool("write")),
            "tracker_disconnect" => self.tracker_disconnect(ctx),
            "tracker_sync" => self.tracker_sync(ctx),
            "tracker_preview" => self.tracker_preview(&ctx, a),
            "tracker_apply" => self.tracker_apply(&ctx, a.get("changes")),
            "tracker_assigned" => self.tracker_assigned(ctx, a.opt_str("period").unwrap_or("")),
            "tracker_import" => self.tracker_import(&ctx, a.get("ids")),
            "tracker_item" => a.str("id").and_then(|id| self.tracker_item(&ctx, id)),
            "tracker_create_item" => self.tracker_create_item(&ctx, a),
            "tracker_push_states" => self.tracker_push_states(&ctx, a.get("changes"), a.opt_str("secret")),
            "tracker_open" => a.str("id").and_then(|id| {
                let id = clean_id(id);
                let conn = ctx.conn()?;
                if let Kind::Missing(k) = &ctx.kind {
                    return Err(plugin_missing(k).into());
                }
                if !ctx.kind.accepts_id(&id) {
                    return Err(ApiError::new("tracker.badId", &[("id", &id)]));
                }
                wb_platform::open_url(&ctx.kind.item_url(conn, &id)?)?;
                Ok(Json::Null)
            }),
            _ => Err(ApiError::new("error.unknownCommand", &[("name", cmd)])),
        })
    }

    /// Conecta la raíz abierta. Sin `secret` se usa la credencial guardada para esa organización.
    fn tracker_connect(&self, mut ctx: Ctx, a: &Args) -> ApiResult<Json> {
        let kind = a.opt_str("kind").filter(|k| !k.is_empty()).unwrap_or(azure::KIND);
        let (org, project) = (a.opt_str("org").unwrap_or(""), a.opt_str("project").unwrap_or(""));
        let secret = a.opt_str("secret").map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        let resolved = self.tracker_kind(kind);
        let conn = match &resolved {
            Kind::Azure => {
                let conn = Connection::normalized(kind, org, project)?;
                if let Some(secret) = &secret {
                    self.secrets().set(&secret_key(&conn, false), secret).map_err(store_error)?;
                }
                conn
            }
            // El plugin valida y normaliza con la credencial (la escrita o la guardada).
            Kind::Plugin(p) => {
                let (conn, used) = self.plugin_tracker_connect(p, org, project, secret)?;
                self.secrets().set(&secret_key(&conn, false), &used).map_err(store_error)?;
                conn
            }
            Kind::Missing(k) => return Err(ApiError::new("tracker.unknownKind", &[("kind", k)])),
        };
        if !self.has_secret(&conn, false) {
            return Err(ApiError::new("tracker.noSecretFor", &[("org", &conn.org)]));
        }
        ctx.file.connections.insert(ctx.root_key.clone(), conn.clone());
        ctx.conn = Some(conn);
        ctx.kind = resolved;
        self.save_trackers(&ctx)?;
        self.tracker_config(&ctx)
    }

    /// Trae los estados del proceso y los campos del proyecto (prueba la conexión).
    fn tracker_fetch(&self, mut ctx: Ctx) -> ApiResult<Json> {
        let client = self.tracker_client(&ctx, false)?;
        let states = client.states()?;
        if states.is_empty() {
            return Err(ApiError::new("tracker.noStates", &[("source", &self.source(&ctx))]));
        }
        let catalog = ctx.kind.catalog(&client.fields()?);
        drop(client);
        let proc = ctx.project_mut()?;
        proc.states = states;
        proc.fetched_at = Some(now());
        proc.set_fields(catalog);
        self.save_trackers(&ctx)?;
        self.tracker_config(&ctx)
    }

    fn tracker_set_map(&self, mut ctx: Ctx, map: &Json) -> ApiResult<Json> {
        let valid: BTreeSet<String> = ctx.ws.workflow().states.iter().map(|s| s.id.clone()).collect();
        let map: BTreeMap<String, String> = map
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| {
                v.as_str().filter(|v| valid.contains(*v)).map(|v| (k.clone(), v.to_string()))
            })
            .collect();
        let proc = ctx.project_mut()?;
        proc.map = map;
        proc.map_saved = true;
        self.save_trackers(&ctx)?;
        self.tracker_config(&ctx)
    }

    /// Elige el campo de fecha pactada (`due`) o de horas pactadas (`hours`); `""` = ninguno.
    fn tracker_set_field(&self, mut ctx: Ctx, which: &str, reference: Option<&str>) -> ApiResult<Json> {
        let reference = reference.unwrap_or("").trim().to_string();
        let proc = ctx.project_mut()?;
        let (list, slot) = match which {
            "due" => (&proc.date_fields, &mut proc.due_field),
            "hours" => (&proc.num_fields, &mut proc.hours_field),
            _ => return Err(ApiError::new("error.badArgument", &[("name", "which")])),
        };
        if !reference.is_empty() && !list.iter().flatten().any(|f| f.reference == reference) {
            return Err(ApiError::new("tracker.unknownField", &[("field", &reference)]));
        }
        *slot = Some(reference);
        self.save_trackers(&ctx)?;
        self.tracker_config(&ctx)
    }

    fn tracker_clear_secret(&self, ctx: &Ctx, write: bool) -> ApiResult<Json> {
        self.secrets().delete(&secret_key(ctx.conn()?, write)).map_err(store_error)?;
        self.tracker_config(ctx)
    }

    /// Desvincula la raíz y borra su caché. Las credenciales y el mapeo quedan (por organización y
    /// proyecto) por si se vuelve a conectar.
    fn tracker_disconnect(&self, mut ctx: Ctx) -> ApiResult<Json> {
        ctx.file.connections.remove(&ctx.root_key);
        ctx.conn = None;
        ctx.kind = Kind::Azure;
        self.save_trackers(&ctx)?;
        self.write_cache(&ctx.root_key, None)?;
        self.tracker_config(&ctx)
    }

    /// Lee todos los work items de la raíz (también los cerrados: un campo recién elegido completa lo
    /// cargado antes) y los deja en la caché. No escribe nada en la carpeta de trabajos.
    fn tracker_sync(&self, mut ctx: Ctx) -> ApiResult<Json> {
        let ids: BTreeSet<String> = ctx.ws.list_items()?.into_iter().flat_map(|it| it.ids).collect();
        let ids: Vec<String> = ids.into_iter().collect();
        if ids.is_empty() {
            return Err(ApiError::new("tracker.noIds", &[]));
        }
        let client = self.tracker_client(&ctx, false)?;
        // Solo los ids de este tracker (los de un plugin, según su `idPattern`).
        let ids: Vec<String> = match ctx.kind {
            Kind::Azure => ids,
            _ => ids.into_iter().filter(|i| ctx.kind.accepts_id(i)).collect(),
        };
        if ids.is_empty() {
            return Err(ApiError::new("tracker.noIds", &[]));
        }
        let mut proc = ctx.project();
        let mut changed = false;
        // Campos sin leer, o una fecha pactada que no se pudo adivinar (como la v1, se reintenta en cada
        // sync hasta adivinarla o hasta que el usuario elija una, aunque sea «ninguna»).
        if proc.date_fields.is_none() || proc.num_fields.is_none() || proc.due_field.is_none() {
            proc.set_fields(ctx.kind.catalog(&client.fields()?));
            changed = true;
        }
        // Un plugin dice cuál es el campo de horas (`role`): sin él, ninguno.
        if proc.hours_field.is_none() && !matches!(ctx.kind, Kind::Azure) {
            proc.hours_field = Some(String::new());
            changed = true;
        }
        if proc.hours_field.is_none() {
            let nums: Vec<FieldInfo> = proc
                .num_fields
                .iter()
                .flatten()
                .map(|f| FieldInfo {
                    reference: f.reference.clone(),
                    name: f.name.clone(),
                    kind: String::new(),
                    role: None,
                })
                .collect();
            let cands = azure::hours_candidates(&nums);
            proc.hours_field = Some(if cands.is_empty() {
                String::new()
            } else {
                let sample =
                    client.work_items(&ids, &FieldRequest { extra: cands.clone(), ..Default::default() })?;
                azure::pick_hours(&cands, &sample)
            });
            changed = true;
        }
        if changed {
            *ctx.project_mut()? = proc.clone();
            self.save_trackers(&ctx)?;
        }
        let items = client.work_items(&ids, &proc.request())?;
        let conn = ctx.conn()?;
        let entry = CacheEntry {
            source: self.source(&ctx),
            kind: conn.kind.clone(),
            org: conn.org.clone(),
            project: conn.project.clone(),
            synced_at: Some(now()),
            requested: ids,
            items: items.into_iter().map(|(k, v)| (k, v.for_cache())).collect(),
        };
        self.write_cache(&ctx.root_key, Some(&entry))?;
        self.tracker_config(&ctx)
    }

    fn tracker_preview(&self, ctx: &Ctx, a: &Args) -> ApiResult<Json> {
        let proc = ctx.project();
        let cache = self.read_cache(&ctx.root_key);
        let only: Option<BTreeSet<String>> =
            a.get("only").as_array().map(|_| str_list(a.get("only")).into_iter().collect());
        let items = ctx.ws.list_items()?;
        let git = self.git_reach(&items);
        Ok(preview(
            ctx.ws.workflow(),
            &items,
            &cache,
            &proc.map,
            only.as_ref(),
            a.bool("includeDiffer"),
            &git,
        ))
    }

    /// Escribe en el README local SOLO el estado y el tipo de cada cambio. Nunca escribe en el tracker.
    fn tracker_apply(&self, ctx: &Ctx, changes: &Json) -> ApiResult<Json> {
        let wf = ctx.ws.workflow();
        let (mut done, mut errors) = (vec![], vec![]);
        for ch in changes.as_array().into_iter().flatten() {
            let name = ch["name"].as_str().unwrap_or("").to_string();
            let mut raw: IndexMap<String, Json> = IndexMap::new();
            for (arg, key) in [("status", &wf.fields.status), ("type", &wf.fields.kind)] {
                if let Some(v) = ch[arg].as_str().filter(|v| !v.is_empty()) {
                    raw.insert(key.clone(), json!(v));
                }
            }
            let res = if raw.is_empty() {
                Err(ApiError::new("tracker.nothingToApply", &[]))
            } else {
                validate_fields(wf, &raw)
                    .map_err(ApiError::from)
                    .and_then(|f| Ok(ctx.ws.update_meta(&name, &f)?))
            };
            match res {
                Ok(_) => done.push(name),
                Err(e) => errors.push(json!({"name": name, "error": e})),
            }
        }
        Ok(json!({"done": done, "errors": errors}))
    }

    /// Work items asignados al dueño de la credencial de lectura con fecha pactada en el período
    /// (`AAAA-MM`; vacío = el actual), sin los cerrados. Marca los que ya tienen carpeta.
    fn tracker_assigned(&self, mut ctx: Ctx, period: &str) -> ApiResult<Json> {
        let client = self.tracker_client(&ctx, false)?;
        if ctx.project().date_fields.is_none() {
            let catalog = ctx.kind.catalog(&client.fields()?);
            ctx.project_mut()?.set_fields(catalog);
            self.save_trackers(&ctx)?;
        }
        let proc = ctx.project();
        let field = proc
            .due_field
            .clone()
            .filter(|f| !f.is_empty())
            .ok_or_else(|| ApiError::new("tracker.noDueField", &[]))?;
        let wf = ctx.ws.workflow();
        let period = match period.trim() {
            "" => {
                let today = chrono::Local::now().date_naive();
                wf.period_key(today.year(), today.month(), today.day())
            }
            p => p.to_string(),
        };
        let (start, end) = period_bounds(wf, &period)
            .ok_or_else(|| ApiError::new("tracker.badPeriod", &[("period", &period)]))?;
        let closed: Vec<String> = proc
            .states
            .iter()
            .filter(|s| s.suggested.is_some_and(StateCategory::is_closed))
            .map(|s| s.name.clone())
            .collect();
        let fmt = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
        let ids = client.assigned_to_me(&field, &fmt(start), &fmt(end), &closed)?;
        let items = if ids.is_empty() {
            BTreeMap::new()
        } else {
            client.work_items(&ids, &FieldRequest { due: Some(field.clone()), ..proc.request() })?
        };
        let mut local: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for it in ctx.ws.list_items()? {
            for id in &it.ids {
                local.entry(id.clone()).or_default().push(it.name.clone());
            }
        }
        let rows: Vec<Json> = ids
            .iter()
            .filter_map(|id| {
                let w = items.get(id)?;
                let mut row = serde_json::to_value(w).ok()?;
                row["id"] = json!(id);
                row["localType"] = json!(local_type(wf, &w.kind));
                row["existing"] = json!(local.get(id).cloned().unwrap_or_default());
                Some(row)
            })
            .collect();
        Ok(json!({
            "period": period,
            "start": fmt(start),
            "end": fmt(end.pred_opt().unwrap_or(end)),
            "field": field,
            "items": rows,
        }))
    }

    /// Crea una carpeta en el estado inicial por work item (título, tipo, descripción y criterios del
    /// tracker). Los que ya tienen carpeta se saltean. Lo leído se suma a la caché.
    fn tracker_import(&self, ctx: &Ctx, ids: &Json) -> ApiResult<Json> {
        let wf = ctx.ws.workflow();
        let ids: Vec<String> = str_list(ids).iter().map(|i| clean_id(i)).collect();
        if ids.is_empty() || !ids.iter().all(|i| ctx.kind.accepts_id(i) && wf.is_valid_item_id(i)) {
            return Err(ApiError::new("tracker.pickItems", &[]));
        }
        let mut have: BTreeSet<String> = ctx.ws.list_items()?.into_iter().flat_map(|it| it.ids).collect();
        let client = self.tracker_client(ctx, false)?;
        let proc = ctx.project();
        let items = client.work_items(&ids, &FieldRequest { text: true, ..proc.request() })?;
        let source = self.source(ctx);
        let (mut done, mut skipped, mut errors) = (vec![], vec![], vec![]);
        for id in &ids {
            if have.contains(id) {
                skipped.push(id.clone());
                continue;
            }
            let Some(w) = items.get(id) else {
                errors.push(json!({"id": id, "error": ApiError::new("tracker.itemMissing", &[("id", id)])}));
                continue;
            };
            let new = NewItem {
                id: id.clone(),
                title: w.title.clone(),
                local_type: local_type(wf, &w.kind),
                folder: folder_for(wf, id, &w.title),
                description: w.description.clone().unwrap_or_default(),
                criteria: w.criteria.clone().unwrap_or_default(),
                tracker_type: w.kind.clone(),
                tracker_state: w.state.clone(),
                source: source.clone(),
            };
            match self.create_new_item(&ctx.ws, &new, None) {
                Ok(out) => {
                    done.push(out["item"]["name"].as_str().unwrap_or_default().to_string());
                    have.insert(id.clone());
                }
                Err(e) => errors.push(json!({"id": id, "error": e})),
            }
        }
        let conn = ctx.conn()?;
        let mut entry = self.read_cache(&ctx.root_key);
        if entry.org != conn.org || entry.project != conn.project {
            // Nunca sincronizada con esta conexión: «último sync» sigue en nunca.
            entry = CacheEntry {
                source: source.clone(),
                kind: conn.kind.clone(),
                org: conn.org.clone(),
                project: conn.project.clone(),
                ..Default::default()
            };
        }
        for (id, w) in items {
            if !entry.requested.contains(&id) {
                entry.requested.push(id.clone());
            }
            entry.items.insert(id, w.for_cache());
        }
        entry.requested.sort();
        self.write_cache(&ctx.root_key, Some(&entry))?;
        Ok(json!({"done": done, "skipped": skipped, "errors": errors}))
    }

    /// Datos de un work item para «Nuevo item». Sin conexión, sin credencial o con una credencial
    /// rechazada vuelve vacío, para completarlo a mano (como la v1); `error` dice por qué.
    fn tracker_item(&self, ctx: &Ctx, id: &str) -> ApiResult<Json> {
        let wf = ctx.ws.workflow();
        let id = clean_id(id);
        if !wf.is_valid_item_id(&id) {
            return Err(ApiError::new("tracker.badId", &[("id", &id)]));
        }
        let existing: Vec<String> =
            ctx.ws.list_items()?.into_iter().filter(|it| it.ids.contains(&id)).map(|it| it.name).collect();
        let connected = ctx.conn.as_ref().is_some_and(|c| self.has_secret(c, false));
        let mut error = Json::Null;
        let detail = if let (true, Kind::Missing(k)) = (ctx.conn.is_some(), &ctx.kind) {
            error = json!(ApiError::from(plugin_missing(k)));
            None
        } else if connected && ctx.kind.accepts_id(&id) {
            match self.tracker_client(ctx, false).and_then(|c| Ok(c.detail(&id)?)) {
                Ok(d) => Some(d),
                Err(e) if MANUAL_ON.contains(&e.code.as_str()) => {
                    error = json!(e);
                    None
                }
                Err(e) => return Err(e),
            }
        } else {
            None
        };
        let d = detail.clone().unwrap_or_else(|| wb_tracker::Detail { id: id.clone(), ..Default::default() });
        Ok(json!({
            "id": id,
            "title": d.title,
            "type": d.kind,
            "localType": local_type(wf, &d.kind),
            "state": d.state,
            "description": d.description,
            "criteria": d.criteria,
            "folder": folder_for(wf, &id, &d.title),
            "existing": existing,
            "fromTracker": detail.is_some(),
            "source": self.source(ctx),
            "error": error,
        }))
    }

    /// Nuevo item con id del tracker: la carpeta con su README (estado inicial, id, tipo, línea de origen,
    /// descripción, criterios, plan y tareas). Con `plan`, en el estado de planificación y con la sesión
    /// de Planear abierta. Lo único que se escribe en la carpeta de trabajos. Devuelve `{item, session}`.
    fn tracker_create_item(&self, ctx: &Ctx, a: &Args) -> ApiResult<Json> {
        let wf = ctx.ws.workflow();
        let id = clean_id(a.str("id")?);
        if !wf.is_valid_item_id(&id) {
            return Err(ApiError::new("tracker.badId", &[("id", &id)]));
        }
        let local_type = a.opt_str("type").unwrap_or("").to_string();
        if !local_type.is_empty() && !wf.types.iter().any(|t| t.id == local_type) {
            return Err(ApiError::new("error.badArgument", &[("name", "type")]));
        }
        let s = |k: &str| a.opt_str(k).unwrap_or("").trim().to_string();
        let title = squash(&s("title"));
        let folder = match s("folder") {
            f if f.is_empty() => folder_for(wf, &id, &title),
            f => f,
        };
        let new = NewItem {
            id,
            title,
            local_type,
            folder,
            description: s("description"),
            criteria: s("criteria"),
            tracker_type: s("trackerType"),
            tracker_state: s("trackerState"),
            source: self.source(ctx),
        };
        self.create_new_item(&ctx.ws, &new, a.bool("plan").then_some(a))
    }

    /// ESCRIBE en el tracker: cambia el estado de cada work item (`[{id, state}]`). Usa la credencial de
    /// escritura (si viene `secret`, se guarda antes). Actualiza la caché con lo que devolvió el tracker.
    fn tracker_push_states(&self, ctx: &Ctx, changes: &Json, secret: Option<&str>) -> ApiResult<Json> {
        let conn = ctx.conn()?;
        // Sin credencial de escritura aparte, se escribe con la de lectura (y `secret` no se usa).
        if let Some(secret) = secret.map(str::trim).filter(|s| !s.is_empty() && ctx.kind.write_secret()) {
            self.secrets().set(&secret_key(conn, true), secret).map_err(store_error)?;
        }
        let client = self.tracker_client(ctx, true)?;
        let known: BTreeSet<String> = ctx.project().states.into_iter().map(|s| s.name).collect();
        let mut entry = self.read_cache(&ctx.root_key);
        let (mut done, mut errors) = (vec![], vec![]);
        for ch in changes.as_array().into_iter().flatten() {
            let id = clean_id(ch["id"].as_str().unwrap_or(""));
            let state = ch["state"].as_str().unwrap_or("").trim().to_string();
            let res = if !ctx.kind.accepts_id(&id) {
                Err(ApiError::new("tracker.badId", &[("id", &id)]))
            } else if state.is_empty() || (!known.is_empty() && !known.contains(&state)) {
                Err(ApiError::new("tracker.unknownState", &[("state", &state)]))
            } else {
                client.set_state(&id, &state).map_err(ApiError::from)
            };
            match res {
                Ok(r) => {
                    if let Some(w) = entry.items.get_mut(&id) {
                        w.state = r.state.clone();
                        if !r.changed.is_empty() {
                            w.changed = r.changed.clone();
                        }
                    }
                    done.push(json!({"id": id, "state": r.state}));
                }
                Err(e) => errors.push(json!({"id": id, "error": e})),
            }
        }
        if !done.is_empty() && !entry.source.is_empty() {
            self.write_cache(&ctx.root_key, Some(&entry))?;
        }
        Ok(json!({"done": done, "errors": errors}))
    }
}

/// Errores del tracker con los que «Traer» sigue en manual (como el `PermissionError` de la v1).
const MANUAL_ON: &[&str] =
    &["tracker.noSecret", "tracker.auth", "tracker.loginPage", "tracker.pluginAuth", "tracker.pluginMissing"];

/// Carpeta sugerida: la plantilla del flujo con el id y el título.
fn folder_for(wf: &Workflow, id: &str, title: &str) -> String {
    let slug = if title.trim().is_empty() { String::new() } else { slugify(title, 40, "item") };
    wf.folders.new_item_template.replace("{id}", id).replace("{slug}", &slug).trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_adds_an_unchecked_git_row_only_when_it_advances() {
        let base = std::env::temp_dir().join(format!("wb-trk-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for (folder, estado) in [
            ("wi111111-a", "codeado"),
            ("wi222222-b", "prod"),
            ("wi333333-c", "descartado"),
            ("wi444444-d", ""),
        ] {
            std::fs::create_dir_all(base.join(folder)).unwrap();
            let fm = if estado.is_empty() {
                String::new()
            } else {
                format!(
                    "---
estado: {estado}
---
"
                )
            };
            std::fs::write(
                base.join(folder).join("README.md"),
                format!(
                    "{fm}# {folder}
"
                ),
            )
            .unwrap();
        }
        let wf = Workflow::preset("classic-es").unwrap();
        let ws = Workspace::open(&base, wf.clone(), None).unwrap();
        let items = ws.list_items().unwrap();
        let mut cache = CacheEntry::default();
        for id in ["111111", "222222", "333333", "444444"] {
            cache.items.insert(
                id.into(),
                WorkItem { state: "Active".into(), kind: "Bug".into(), ..Default::default() },
            );
        }
        let map = BTreeMap::from([("Active".to_string(), "en-curso".to_string())]);
        let git: HashMap<String, (String, Vec<String>)> =
            ["wi111111-a", "wi222222-b", "wi333333-c", "wi444444-d"]
                .iter()
                .map(|n| (n.to_string(), ("qa".to_string(), vec!["app".to_string()])))
                .collect();
        let out = preview(&wf, &items, &cache, &map, None, false, &git);
        let rows: Vec<&Json> =
            out["rows"].as_array().unwrap().iter().filter(|r| r["kind"] == "git").collect();
        // codeado → qa avanza; prod ya está más adelante; descartado no se toca; vacío → propone qa.
        let names: Vec<&str> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["wi111111-a", "wi444444-d"]);
        assert_eq!(rows[0]["status"], "qa");
        assert_eq!(rows[0]["key"], "wi111111-a|git");
        assert_eq!(rows[0]["on"], false);
        assert_eq!(rows[0]["why"], json!({"env": "qa", "repos": ["app"]}));
        // Las filas de git van al final.
        assert_eq!(out["rows"].as_array().unwrap().last().unwrap()["kind"], "git");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Responde siempre lo mismo (un 401 = credencial rechazada).
    struct Fixed(u16);

    impl Transport for Fixed {
        fn send(&self, _req: &wb_tracker::HttpRequest) -> Result<wb_tracker::HttpResponse, String> {
            Ok(wb_tracker::HttpResponse {
                status: self.0,
                content_type: "application/json".into(),
                body: b"{}".to_vec(),
            })
        }
    }

    #[test]
    fn fetch_with_a_rejected_secret_stays_manual_and_creates_the_v1_readme() {
        let base = std::env::temp_dir().join(format!("wb-trk-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("work")).unwrap();
        let api = Api::new(base.join("cfg"));
        api.set_tracker_backend(
            Arc::new(wb_platform::secrets::MemorySecrets::default()),
            Arc::new(Fixed(401)),
        );
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": base.join("work")})).unwrap();
        api.dispatch(
            "tracker_connect",
            &json!({"org": "https://dev.azure.com/acme", "project": "Shop", "secret": "bad"}),
        )
        .unwrap();

        // v1 `plan_wi_fetch`: PAT inválido = se completa a mano.
        let d = api.dispatch("tracker_item", &json!({"id": "#123456"})).unwrap();
        assert_eq!(d["fromTracker"], false);
        assert_eq!(d["error"]["code"], "tracker.auth");
        assert_eq!(d["folder"], "wi123456");

        let out = api
            .dispatch(
                "tracker_create_item",
                &json!({"id": "123456", "title": "Login  SSO", "type": "bug", "description": "Falla",
                        "criteria": "", "trackerType": "Bug", "trackerState": "Active"}),
            )
            .unwrap();
        assert_eq!(out["item"]["name"], "wi123456-login-sso");
        assert_eq!(out["session"], Json::Null);
        let text = std::fs::read_to_string(base.join("work/wi123456-login-sso/README.md")).unwrap();
        assert_eq!(
            text,
            "---\nestado: pendiente\nwi: 123456\ntipo: bug\n---\n# Login SSO\n\n> Bug #123456 · Azure: Active\n\n\
             ## Descripción\n\nFalla\n\n## Plan\n\n*(lo completa la sesión de planificación)*\n\n## Pendientes\n\n"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
