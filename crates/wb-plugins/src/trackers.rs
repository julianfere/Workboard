//! Trackers que aporta un plugin (`trackers` del manifiesto, ver `docs/plugins.md` → *Trackers*): el
//! formulario de conexión, la página de un item y qué ids le pertenecen. Las llamadas (`tracker/*`) las
//! hace `wb-api`, que conoce a la vez el trait `Tracker` y `rpc::call`.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, Plugin, Plugins, Text};

static KIND_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9-]{1,40}$").expect("kind regex"));
/// Tipos de los trackers incluidos (y el nombre corto con el que se los conoce): un plugin no puede usarlos.
pub const RESERVED_KINDS: [&str; 2] = ["azure", "azure-devops"];

/// Un campo de la conexión (organización o proyecto).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnField {
    #[serde(default)]
    pub label: Option<Text>,
    #[serde(default)]
    pub placeholder: Option<Text>,
    /// Solo para `org` (`project` es siempre obligatorio). Por defecto, obligatorio.
    #[serde(default)]
    pub required: Option<bool>,
}

/// El campo de la credencial.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretField {
    #[serde(default)]
    pub label: Option<Text>,
    #[serde(default)]
    pub hint: Option<Text>,
}

/// Un tracker que aporta un plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrackerDecl {
    pub kind: String,
    pub name: Text,
    #[serde(default)]
    pub org: ConnField,
    #[serde(default)]
    pub project: ConnField,
    #[serde(default)]
    pub secret: SecretField,
    /// Las escrituras usan una credencial aparte (como el PAT de escritura de Azure).
    #[serde(default)]
    pub write_secret: bool,
    /// Página de un item: `https://…` con `{org}`, `{project}` e `{id}`.
    pub item_url: String,
    /// Qué ids de la carpeta son de este tracker (sin él, cualquiera).
    #[serde(default)]
    pub id_pattern: Option<String>,
}

/// Codifica un valor para la URL (todo menos los caracteres no reservados).
fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl TrackerDecl {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !KIND_RE.is_match(&self.kind) {
            return Err(format!("tracker kind {:?}: use a-z, 0-9 and -", self.kind));
        }
        if RESERVED_KINDS.contains(&self.kind.as_str()) {
            return Err(format!("tracker kind {:?} is built in", self.kind));
        }
        let rest = self.item_url.strip_prefix("https://").unwrap_or_default();
        if rest.is_empty() || rest.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\') {
            return Err(format!("tracker {}: itemUrl must be https://", self.kind));
        }
        if let Some(p) = &self.id_pattern {
            Regex::new(p).map_err(|e| format!("tracker {}: idPattern: {e}", self.kind))?;
        }
        Ok(())
    }

    /// `org` puede quedar vacío.
    pub fn org_optional(&self) -> bool {
        self.org.required == Some(false)
    }

    /// Si un id de la carpeta es de este tracker.
    pub fn accepts_id(&self, id: &str) -> bool {
        if id.is_empty() {
            return false;
        }
        match &self.id_pattern {
            None => true,
            Some(p) => Regex::new(p).is_ok_and(|re| re.is_match(id)),
        }
    }

    /// Página del item (los valores van codificados; el resultado sigue siendo `https://`).
    pub fn item_url(&self, org: &str, project: &str, id: &str) -> Option<String> {
        let url = self
            .item_url
            .replace("{org}", &pct(org))
            .replace("{project}", &pct(project))
            .replace("{id}", &pct(id));
        url.starts_with("https://").then_some(url)
    }
}

/// Valida los `trackers` de un manifiesto (tipos válidos y no repetidos dentro del plugin).
pub(crate) fn validate(trackers: &[TrackerDecl]) -> Result<(), String> {
    let mut kinds = BTreeSet::new();
    for t in trackers {
        t.validate()?;
        if !kinds.insert(t.kind.as_str()) {
            return Err(format!("repeated tracker kind {}", t.kind));
        }
    }
    Ok(())
}

/// Un tracker de un plugin habilitado.
#[derive(Debug, Clone)]
pub struct PluginTracker<'a> {
    pub plugin: &'a Plugin,
    pub manifest: &'a Manifest,
    pub decl: &'a TrackerDecl,
}

/// Un tracker descartado porque su tipo ya lo usa otro plugin habilitado (antes en el orden de carpetas).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerClash {
    /// El plugin cuyo tracker quedó afuera.
    pub plugin: String,
    pub kind: String,
    /// El plugin que ya lo tenía.
    pub owner: String,
}

impl Plugins {
    fn tracker_results(&self) -> (Vec<PluginTracker<'_>>, Vec<TrackerClash>) {
        let (mut ok, mut clashes) = (Vec::<PluginTracker>::new(), vec![]);
        for (plugin, manifest) in self.enabled() {
            for decl in &manifest.trackers {
                match ok.iter().find(|t| t.decl.kind == decl.kind) {
                    Some(t) => clashes.push(TrackerClash {
                        plugin: manifest.id.clone(),
                        kind: decl.kind.clone(),
                        owner: t.manifest.id.clone(),
                    }),
                    None => ok.push(PluginTracker { plugin, manifest, decl }),
                }
            }
        }
        (ok, clashes)
    }

    /// Los trackers de los plugins habilitados (sin los de tipo repetido).
    pub fn trackers(&self) -> Vec<PluginTracker<'_>> {
        self.tracker_results().0
    }

    /// Trackers descartados por repetir el tipo de otro plugin habilitado.
    pub fn tracker_clashes(&self) -> Vec<TrackerClash> {
        self.tracker_results().1
    }

    /// El tracker habilitado de tipo `kind`.
    pub fn tracker(&self, kind: &str) -> Option<PluginTracker<'_>> {
        self.trackers().into_iter().find(|t| t.decl.kind == kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(json: serde_json::Value) -> Result<TrackerDecl, String> {
        let d: TrackerDecl = serde_json::from_value(json).map_err(|e| e.to_string())?;
        d.validate()?;
        Ok(d)
    }

    #[test]
    fn declarations_are_validated() {
        let ok = decl(
            serde_json::json!({"kind": "trello", "name": "Trello", "itemUrl": "https://trello.com/c/{id}",
            "idPattern": "^[0-9]+$", "org": {"required": false}}),
        )
        .unwrap();
        assert!(ok.org_optional() && !ok.write_secret);
        assert!(ok.accepts_id("12") && !ok.accepts_id("AB-1") && !ok.accepts_id(""));
        assert_eq!(ok.item_url("a b", "p", "1/2").unwrap(), "https://trello.com/c/1%2F2");
        let base = |k: &str, url: &str, pat: Option<&str>| {
            decl(serde_json::json!({"kind": k, "name": "X", "itemUrl": url, "idPattern": pat}))
        };
        assert!(base("jira", "https://{org}.atlassian.net/browse/{id}", None).is_ok());
        for (k, url, pat) in [
            ("Jira", "https://x", None),
            ("azure", "https://x", None),
            ("azure-devops", "https://x", None),
            ("jira", "http://x/{id}", None),
            ("jira", "https://", None),
            ("jira", "javascript:alert(1)", None),
            ("jira", "https://x", Some("([")),
        ] {
            assert!(base(k, url, pat).is_err(), "{k} {url} {pat:?}");
        }
        assert!(
            decl(serde_json::json!({"kind": "x", "name": "X", "itemUrl": "https://x", "other": 1})).is_err()
        );
    }
}
