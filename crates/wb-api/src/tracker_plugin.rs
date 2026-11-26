//! Trackers de plugins (`trackers` del manifiesto): el tipo de cada conexión (Azure incluido, un plugin
//! habilitado o uno que falta) y el cliente que implementa [`Tracker`] llamando a `tracker/*` del plugin,
//! un proceso por llamada (`docs/plugins.md` → *Trackers*).
//!
//! La credencial viaja solo por el stdin del plugin y se quita de todo texto que vuelve (errores y stderr).

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::{Value as Json, json};
use wb_plugins::{Manifest, Plugin, PluginError, Plugins, TrackerDecl};
use wb_tracker::azure::{self, Catalog};
use wb_tracker::{
    Connection, Detail, FieldInfo, FieldRequest, Result, StateChange, Tracker, TrackerError, TrackerState,
    WorkItem,
};

use crate::{Api, ApiError, ApiResult};

/// Papeles de un campo que entiende la app.
const ROLES: [&str; 4] = ["due", "hours", "priority", "effort"];
/// Ids por llamada a `tracker/workItems` (como los lotes de Azure).
const BATCH: usize = 200;

/// Un tracker de un plugin habilitado (copiado del manifiesto).
#[derive(Debug, Clone)]
pub(crate) struct PluginKind {
    pub(crate) plugin: Plugin,
    pub(crate) manifest: Manifest,
    pub(crate) decl: TrackerDecl,
}

/// De qué es una conexión.
#[derive(Debug, Clone)]
pub(crate) enum Kind {
    Azure,
    Plugin(Box<PluginKind>),
    /// Un tipo que no es de la app ni de un plugin habilitado (deshabilitado, borrado o repetido).
    Missing(String),
}

pub(crate) fn plugin_missing(kind: &str) -> TrackerError {
    TrackerError::new("tracker.pluginMissing", vec![("kind", kind.to_string())])
}

impl Kind {
    pub(crate) fn of(plugins: &Plugins, kind: &str) -> Self {
        if kind == azure::KIND {
            return Self::Azure;
        }
        match plugins.tracker(kind) {
            Some(t) => Self::Plugin(Box::new(PluginKind {
                plugin: t.plugin.clone(),
                manifest: t.manifest.clone(),
                decl: t.decl.clone(),
            })),
            None => Self::Missing(kind.to_string()),
        }
    }

    /// Nombre para mostrar (en un plugin que falta, su tipo).
    pub(crate) fn source(&self, lang: &str) -> String {
        match self {
            Self::Azure => azure::SOURCE.to_string(),
            Self::Plugin(p) => p.decl.name.get(lang),
            Self::Missing(k) => k.clone(),
        }
    }

    /// Si un id de la carpeta es de este tracker (Azure: numérico; un plugin: su `idPattern`).
    pub(crate) fn accepts_id(&self, id: &str) -> bool {
        match self {
            Self::Azure => !id.is_empty() && id.len() <= 10 && id.bytes().all(|b| b.is_ascii_digit()),
            Self::Plugin(p) => p.decl.accepts_id(id),
            Self::Missing(_) => false,
        }
    }

    /// Las escrituras usan una credencial aparte.
    pub(crate) fn write_secret(&self) -> bool {
        match self {
            Self::Azure | Self::Missing(_) => true,
            Self::Plugin(p) => p.decl.write_secret,
        }
    }

    pub(crate) fn catalog(&self, fields: &[FieldInfo]) -> Catalog {
        match self {
            Self::Azure => azure::catalog(fields),
            _ => wb_tracker::role_catalog(fields),
        }
    }

    pub(crate) fn item_url(&self, conn: &Connection, id: &str) -> Result<String> {
        match self {
            Self::Azure => Ok(conn.item_url(id)),
            Self::Plugin(p) => p
                .decl
                .item_url(&conn.org, &conn.project, id)
                .ok_or_else(|| TrackerError::new("tracker.badId", vec![("id", id.to_string())])),
            Self::Missing(k) => Err(plugin_missing(k)),
        }
    }
}

/// Saca la credencial (y sus partes, para formatos como `key:token`) de un texto.
pub(crate) fn scrub(text: &str, secret: &str) -> String {
    let secret = secret.trim();
    let mut parts: Vec<&str> = secret.split([':', ' ', '\t']).filter(|p| p.len() >= 6).collect();
    parts.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let mut out = text.to_string();
    for s in std::iter::once(secret).filter(|s| s.len() >= 4).chain(parts) {
        out = out.replace(s, "***");
    }
    out
}

/// Si `msg` empieza con el código `code` (solo, o seguido de `:` o un espacio), el resto.
fn after_code<'m>(msg: &'m str, code: &str) -> Option<&'m str> {
    let rest = msg.strip_prefix(code)?;
    (rest.is_empty() || rest.starts_with([':', ' '])).then(|| rest.trim_start_matches(':').trim())
}

/// Un tracker de un plugin con una credencial.
pub(crate) struct PluginClient<'a> {
    pub(crate) api: &'a Api,
    pub(crate) kind: PluginKind,
    pub(crate) conn: Connection,
    pub(crate) secret: String,
    pub(crate) lang: &'static str,
}

impl PluginClient<'_> {
    fn source(&self) -> String {
        self.kind.decl.name.get(self.lang)
    }

    fn error(&self, e: PluginError) -> TrackerError {
        let detail = scrub(&e.detail, &self.secret).trim().to_string();
        let source = self.source();
        if e.code != "plugin.error.failed" {
            return TrackerError::new(e.code, vec![("plugin", source), ("detail", detail)]);
        }
        if let Some(rest) = after_code(&detail, "tracker.auth") {
            return TrackerError::new(
                "tracker.pluginAuth",
                vec![("source", source), ("detail", rest.to_string())],
            );
        }
        if let Some(rest) = after_code(&detail, "tracker.notFound") {
            return TrackerError::new(
                "tracker.projectNotFound",
                vec![
                    ("project", self.conn.project.clone()),
                    ("org", if self.conn.org.is_empty() { source } else { self.conn.org.clone() }),
                    ("detail", rest.to_string()),
                ],
            );
        }
        TrackerError::new("tracker.pluginError", vec![("source", source), ("message", detail)])
    }

    /// Llama a `method` con la conexión y la credencial más `extra`.
    pub(crate) fn call(&self, method: &str, extra: Json) -> Result<Json> {
        let mut params = json!({
            "kind": self.kind.decl.kind,
            "connection": {"org": self.conn.org, "project": self.conn.project},
            "secret": self.secret,
        });
        if let Some(obj) = extra.as_object() {
            for (k, v) in obj {
                params[k] = v.clone();
            }
        }
        self.api
            .plugin_tracker_call(
                &self.kind.plugin,
                &self.kind.manifest,
                self.lang,
                method,
                params,
                &self.secret,
            )
            .map_err(|e| self.error(e))
    }

    fn parse<T: for<'de> Deserialize<'de>>(&self, v: Json) -> Result<T> {
        serde_json::from_value(v).map_err(|e| {
            TrackerError::new("tracker.badResponse", vec![("message", scrub(&e.to_string(), &self.secret))])
        })
    }

    fn id_text(v: &Json) -> Option<String> {
        v.as_str().map(|s| s.trim().to_string()).or_else(|| v.as_u64().map(|n| n.to_string()))
    }

    fn bad_id(id: &str) -> TrackerError {
        TrackerError::new("tracker.badId", vec![("id", id.to_string())])
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct DetailOut {
    id: Option<Json>,
    title: String,
    #[serde(rename = "type")]
    kind: String,
    state: String,
    description: String,
    criteria: String,
}

impl Tracker for PluginClient<'_> {
    fn states(&self) -> Result<Vec<TrackerState>> {
        self.parse(self.call("tracker/states", json!({}))?)
    }

    fn fields(&self) -> Result<Vec<FieldInfo>> {
        let mut fields: Vec<FieldInfo> = self.parse(self.call("tracker/fields", json!({}))?)?;
        fields.retain(|f| !f.reference.is_empty());
        for f in &mut fields {
            if f.name.is_empty() {
                f.name = f.reference.clone();
            }
            f.role = f.role.take().filter(|r| ROLES.contains(&r.as_str()));
        }
        Ok(fields)
    }

    fn work_items(&self, ids: &[String], req: &FieldRequest) -> Result<BTreeMap<String, WorkItem>> {
        let ids: Vec<String> = ids
            .iter()
            .map(|i| i.trim().to_string())
            .filter(|i| self.kind.decl.accepts_id(i))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let request = json!({
            "priority": req.priority, "due": req.due, "hours": req.hours, "effort": req.effort,
            "text": req.text, "extra": req.extra,
        });
        let mut out = BTreeMap::new();
        for chunk in ids.chunks(BATCH) {
            let got: BTreeMap<String, WorkItem> =
                self.parse(self.call("tracker/workItems", json!({"ids": chunk, "request": request}))?)?;
            out.extend(got.into_iter().filter(|(id, _)| chunk.contains(id)));
        }
        Ok(out)
    }

    fn assigned_to_me(
        &self,
        due_field: &str,
        start: &str,
        end: &str,
        exclude: &[String],
    ) -> Result<Vec<String>> {
        let got = self.call(
            "tracker/assignedToMe",
            json!({"dueField": due_field, "start": start, "end": end, "exclude": exclude}),
        )?;
        let list = got.as_array().ok_or_else(|| {
            TrackerError::new(
                "tracker.badResponse",
                vec![("message", "assignedToMe: not a list".to_string())],
            )
        })?;
        let mut seen = BTreeSet::new();
        Ok(list
            .iter()
            .filter_map(Self::id_text)
            .filter(|i| self.kind.decl.accepts_id(i) && seen.insert(i.clone()))
            .collect())
    }

    fn detail(&self, id: &str) -> Result<Detail> {
        if !self.kind.decl.accepts_id(id) {
            return Err(Self::bad_id(id));
        }
        let d: DetailOut = self.parse(self.call("tracker/detail", json!({"id": id}))?)?;
        Ok(Detail {
            id: d
                .id
                .as_ref()
                .and_then(Self::id_text)
                .filter(|i| !i.is_empty())
                .unwrap_or_else(|| id.to_string()),
            title: d.title,
            kind: d.kind,
            state: d.state,
            description: d.description,
            criteria: d.criteria,
        })
    }

    fn set_state(&self, id: &str, state: &str) -> Result<StateChange> {
        if !self.kind.decl.accepts_id(id) {
            return Err(Self::bad_id(id));
        }
        let r = self.call("tracker/setState", json!({"id": id, "state": state}))?;
        Ok(StateChange {
            state: r["state"].as_str().filter(|s| !s.is_empty()).unwrap_or(state).to_string(),
            changed: r["changed"].as_str().unwrap_or_default().to_string(),
        })
    }
}

impl Api {
    /// El tipo de una conexión con los plugins habilitados ahora.
    pub(crate) fn tracker_kind(&self, kind: &str) -> Kind {
        if kind == azure::KIND {
            return Kind::Azure;
        }
        Kind::of(&Plugins::load(&self.config_dir), kind)
    }

    /// Los trackers que se pueden conectar, para el formulario. En el incluido, los textos son claves de
    /// `locales/*.json`; en los de plugins, el texto del manifiesto tal cual (uno solo o `{en, es}`, que
    /// elige la UI con su idioma; `null` = el genérico).
    pub(crate) fn tracker_kinds(&self) -> ApiResult<Vec<Json>> {
        let mut out = vec![json!({
            "kind": azure::KIND,
            "name": azure::SOURCE,
            "org": {"label": "tracker.org", "placeholder": "tracker.orgPlaceholder", "required": true},
            "project": {"label": "tracker.project", "placeholder": null},
            "secret": {"label": "tracker.secret", "hint": "tracker.secretHelp"},
            "writeSecret": true,
            "builtin": true,
            "plugin": null,
        })];
        let plugins = Plugins::load(&self.config_dir);
        for t in plugins.trackers() {
            let d = t.decl;
            out.push(json!({
                "kind": d.kind,
                "name": d.name,
                "org": {"label": d.org.label, "placeholder": d.org.placeholder, "required": !d.org_optional()},
                "project": {"label": d.project.label, "placeholder": d.project.placeholder},
                "secret": {"label": d.secret.label, "hint": d.secret.hint},
                "writeSecret": d.write_secret,
                "builtin": false,
                "plugin": t.manifest.id,
            }));
        }
        Ok(out)
    }

    /// Conexión de un tracker de plugin: recorta, pide la credencial y deja que el plugin normalice
    /// (`tracker/connect`). Devuelve la conexión y la credencial usada.
    pub(crate) fn plugin_tracker_connect(
        &self,
        kind: &PluginKind,
        org: &str,
        project: &str,
        secret: Option<String>,
    ) -> ApiResult<(Connection, String)> {
        let raw = Connection {
            kind: kind.decl.kind.clone(),
            org: org.trim().into(),
            project: project.trim().into(),
        };
        if raw.project.is_empty() {
            return Err(ApiError::new("tracker.noProject", &[]));
        }
        if raw.org.is_empty() && !kind.decl.org_optional() {
            return Err(ApiError::new("tracker.noOrg", &[]));
        }
        let secret = match secret {
            Some(s) => s,
            None => {
                let stored = self.secrets().get(&crate::tracker::secret_key(&raw, false));
                stored.ok().flatten().ok_or_else(|| {
                    let who = if raw.org.is_empty() { &raw.project } else { &raw.org };
                    ApiError::new("tracker.noSecretFor", &[("org", who)])
                })?
            }
        };
        let client = PluginClient {
            api: self,
            kind: kind.clone(),
            conn: raw.clone(),
            secret: secret.clone(),
            lang: self.lang(),
        };
        let out = client.call("tracker/connect", json!({}))?;
        let pick = |k: &str, d: &str| out[k].as_str().map(str::trim).unwrap_or(d).to_string();
        let conn = Connection {
            kind: raw.kind.clone(),
            org: pick("org", &raw.org),
            project: pick("project", &raw.project),
        };
        if conn.project.is_empty() {
            return Err(ApiError::new("tracker.noProject", &[]));
        }
        Ok((conn, secret))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_removes_the_secret_and_its_parts() {
        assert_eq!(scrub("bad key abcdef123:tok-987654 here", "abcdef123:tok-987654"), "bad key *** here");
        assert_eq!(scrub("only abcdef123 leaked", "abcdef123:tok-987654"), "only *** leaked");
        assert_eq!(scrub("short ab", "ab"), "short ab");
        assert_eq!(after_code("tracker.auth: expired", "tracker.auth"), Some("expired"));
        assert_eq!(after_code("tracker.auth", "tracker.auth"), Some(""));
        assert_eq!(after_code("tracker.authx", "tracker.auth"), None);
    }
}
