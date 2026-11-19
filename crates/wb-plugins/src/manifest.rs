//! `manifest.json` de un plugin, la lista de plugins instalados (`<config>/plugins/*/`) y cuáles están
//! habilitados (`<config>/plugins.json`).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use wb_core::Workflow;

/// Versión del protocolo que habla esta app.
pub const PROTOCOL: u32 = 1;
/// Lo que un plugin puede pedirle a la app (ver `docs/plugins.md`).
pub const PERMISSIONS: [&str; 4] = ["items.write", "files.read", "secrets", "tracker"];
/// Dónde aparece una acción.
pub const PLACEMENTS: [&str; 4] = ["item", "items", "palette", "settings"];

static ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,39}$").expect("id regex"));
static KEY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_]{0,39}$").expect("key regex"));

/// Texto de un manifiesto: uno solo o por idioma.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Text {
    Plain(String),
    Langs(BTreeMap<String, String>),
}

impl Text {
    /// En `lang`, si no en inglés, si no el primero.
    pub fn get(&self, lang: &str) -> String {
        match self {
            Text::Plain(s) => s.clone(),
            Text::Langs(m) => {
                m.get(lang).or_else(|| m.get("en")).or_else(|| m.values().next()).cloned().unwrap_or_default()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: Text,
    #[serde(default)]
    pub description: Option<Text>,
    #[serde(default)]
    pub version: String,
    pub protocol: u32,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Claves extra del frontmatter que el plugin puede escribir.
    #[serde(default)]
    pub fields: Vec<String>,
    /// Presets de flujo (rutas relativas a la carpeta del plugin).
    #[serde(default)]
    pub presets: Vec<String>,
    #[serde(default)]
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub label: Text,
    #[serde(default)]
    pub hint: Option<Text>,
    #[serde(default)]
    pub icon: Option<String>,
    pub placements: Vec<String>,
    #[serde(default)]
    pub when: Option<When>,
}

/// Condición sobre los items elegidos (todos tienen que cumplirla).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct When {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub has_ids: Option<bool>,
    #[serde(default)]
    pub categories: Vec<String>,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        m.validate()?;
        Ok(m)
    }

    fn validate(&self) -> Result<(), String> {
        if !ID_RE.is_match(&self.id) {
            return Err(format!("id {:?}: use a-z, 0-9 and -", self.id));
        }
        if self.protocol != PROTOCOL {
            return Err(format!("protocol {} (this app speaks {PROTOCOL})", self.protocol));
        }
        if self.command.trim().is_empty() {
            return Err("empty command".into());
        }
        if let Some(t) = self.timeout_seconds
            && !(5..=600).contains(&t)
        {
            return Err("timeoutSeconds must be 5 to 600".into());
        }
        if let Some(p) = self.permissions.iter().find(|p| !PERMISSIONS.contains(&p.as_str())) {
            return Err(format!("unknown permission {p}"));
        }
        if let Some(k) = self.fields.iter().find(|k| !KEY_RE.is_match(k)) {
            return Err(format!("field {k:?}: use a-z, 0-9 and _"));
        }
        if let Some(p) = self.presets.iter().find(|p| !inside(p)) {
            return Err(format!("preset path {p:?} must stay inside the plugin folder"));
        }
        let mut ids = BTreeSet::new();
        for a in &self.actions {
            if !ID_RE.is_match(&a.id) || !ids.insert(a.id.as_str()) {
                return Err(format!("empty, invalid or repeated action id {:?}", a.id));
            }
            if a.placements.is_empty() {
                return Err(format!("action {}: no placements", a.id));
            }
            if let Some(p) = a.placements.iter().find(|p| !PLACEMENTS.contains(&p.as_str())) {
                return Err(format!("action {}: unknown placement {p}", a.id));
            }
            if let Some(k) = a.when.as_ref().and_then(|w| w.kind.as_deref())
                && !["item", "group"].contains(&k)
            {
                return Err(format!("action {}: when.kind must be item or group", a.id));
            }
        }
        Ok(())
    }

    pub fn action(&self, id: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.id == id)
    }

    pub fn allows(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }

    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.timeout_seconds.unwrap_or(120))
    }
}

/// Ruta relativa sin `..` ni raíz.
fn inside(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty()
        && p.components().all(|c| matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir))
}

/// Un plugin instalado: su carpeta y su manifiesto (o por qué no se pudo leer).
#[derive(Debug, Clone)]
pub struct Plugin {
    pub dir: PathBuf,
    /// Nombre de la carpeta.
    pub folder: String,
    pub manifest: Option<Manifest>,
    pub error: Option<String>,
    pub enabled: bool,
}

impl Plugin {
    /// Línea de comando del plugin: un ejecutable dentro de su carpeta o un programa del `PATH`.
    pub fn argv(&self) -> Result<Vec<OsString>, String> {
        let m = self.manifest.as_ref().ok_or("no manifest")?;
        let cmd = m.command.trim();
        let local = cmd.contains(['/', '\\']) || self.dir.join(cmd).is_file();
        let exe = if local {
            if !inside(cmd) {
                return Err("command must stay inside the plugin folder".into());
            }
            let p = self.dir.join(cmd);
            let with_exe = p.with_extension("exe");
            if p.is_file() {
                p
            } else if cfg!(windows) && p.extension().is_none() && with_exe.is_file() {
                with_exe
            } else {
                return Err(format!("command not found: {}", p.display()));
            }
        } else {
            let path = std::env::var("PATH").unwrap_or_default();
            wb_platform::which(cmd, &path).ok_or_else(|| format!("{cmd} is not on the PATH"))?
        };
        Ok(std::iter::once(exe.into_os_string()).chain(m.args.iter().map(OsString::from)).collect())
    }
}

#[derive(Default, Serialize, Deserialize)]
struct EnabledFile {
    #[serde(default)]
    enabled: Vec<String>,
}

/// Los plugins instalados en una carpeta de config.
#[derive(Debug, Clone)]
pub struct Plugins {
    pub dir: PathBuf,
    pub list: Vec<Plugin>,
}

impl Plugins {
    pub fn dir_in(config_dir: &Path) -> PathBuf {
        config_dir.join("plugins")
    }

    fn enabled_file(config_dir: &Path) -> PathBuf {
        config_dir.join("plugins.json")
    }

    fn read_enabled(config_dir: &Path) -> Vec<String> {
        std::fs::read_to_string(Self::enabled_file(config_dir))
            .ok()
            .and_then(|t| serde_json::from_str::<EnabledFile>(&t).ok())
            .map(|f| f.enabled)
            .unwrap_or_default()
    }

    /// Lee las carpetas de `<config>/plugins/` (en orden). Un id repetido queda con error.
    pub fn load(config_dir: &Path) -> Self {
        let dir = Self::dir_in(config_dir);
        let enabled = Self::read_enabled(config_dir);
        let mut folders: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        folders.sort();
        let mut seen = BTreeSet::new();
        let list = folders
            .into_iter()
            .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(['.', '_'])))
            .map(|p| {
                let folder = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let parsed = std::fs::read_to_string(p.join("manifest.json"))
                    .map_err(|e| format!("manifest.json: {e}"))
                    .and_then(|t| Manifest::parse(&t));
                let (manifest, error) = match parsed {
                    Ok(m) if !seen.insert(m.id.clone()) => (None, Some(format!("repeated id {}", m.id))),
                    Ok(m) => (Some(m), None),
                    Err(e) => (None, Some(e)),
                };
                let on = manifest.as_ref().is_some_and(|m| enabled.contains(&m.id));
                Plugin { dir: p, folder, manifest, error, enabled: on }
            })
            .collect();
        Self { dir, list }
    }

    /// Habilita o deshabilita un plugin por id.
    pub fn set_enabled(config_dir: &Path, id: &str, on: bool) -> std::io::Result<()> {
        let mut list = Self::read_enabled(config_dir);
        list.retain(|x| x != id);
        if on {
            list.push(id.to_string());
        }
        list.sort();
        let text =
            serde_json::to_string_pretty(&EnabledFile { enabled: list }).map_err(std::io::Error::other)?;
        std::fs::create_dir_all(config_dir)?;
        let tmp = Self::enabled_file(config_dir).with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, Self::enabled_file(config_dir))
    }

    /// Un plugin habilitado.
    pub fn get(&self, id: &str) -> Option<(&Plugin, &Manifest)> {
        self.enabled().find(|(_, m)| m.id == id)
    }

    pub fn enabled(&self) -> impl Iterator<Item = (&Plugin, &Manifest)> {
        self.list.iter().filter(|p| p.enabled).filter_map(|p| p.manifest.as_ref().map(|m| (p, m)))
    }

    /// Presets de los plugins habilitados: `(id del plugin, preset)`. Los que no validan o repiten el id de
    /// uno incluido se descartan (salen en `preset_errors`).
    pub fn presets(&self) -> Vec<(String, Workflow)> {
        self.preset_results().into_iter().filter_map(|(plugin, r)| r.ok().map(|w| (plugin, w))).collect()
    }

    pub fn preset_errors(&self) -> Vec<(String, String)> {
        self.preset_results().into_iter().filter_map(|(plugin, r)| r.err().map(|e| (plugin, e))).collect()
    }

    fn preset_results(&self) -> Vec<(String, Result<Workflow, String>)> {
        let mut out = vec![];
        let mut ids: BTreeSet<String> = Workflow::preset_ids().map(String::from).collect();
        for (p, m) in self.enabled() {
            for rel in &m.presets {
                let r = std::fs::read_to_string(p.dir.join(rel))
                    .map_err(|e| format!("{rel}: {e}"))
                    .and_then(|t| Workflow::from_json(&t).map_err(|e| format!("{rel}: {e}")))
                    .and_then(|w| {
                        if ids.insert(w.id.clone()) {
                            Ok(w)
                        } else {
                            Err(format!("{rel}: repeated preset id {}", w.id))
                        }
                    });
                out.push((m.id.clone(), r));
            }
        }
        out
    }
}

/// El flujo `id`: uno incluido o el preset de un plugin habilitado.
pub fn resolve_workflow(config_dir: &Path, id: &str) -> Result<Workflow, wb_core::workflow::WorkflowError> {
    match Workflow::preset(id) {
        Ok(w) => Ok(w),
        Err(e) => {
            Plugins::load(config_dir).presets().into_iter().map(|(_, w)| w).find(|w| w.id == id).ok_or(e)
        }
    }
}

/// `when` de una acción contra un item (en JSON, como lo arma la app).
pub fn matches(when: Option<&When>, wf: &Workflow, item: &Json) -> bool {
    let Some(w) = when else { return true };
    if let Some(k) = &w.kind
        && item["kind"].as_str() != Some(k)
    {
        return false;
    }
    if let Some(h) = w.has_ids
        && item["ids"].as_array().is_some_and(|ids| !ids.is_empty()) != h
    {
        return false;
    }
    if !w.categories.is_empty() {
        let status = item["status"].as_str().unwrap_or("");
        let cat = wf.states.iter().find(|s| s.id == status).map(|s| s.category);
        let cat = cat.and_then(|c| serde_json::to_value(c).ok()).and_then(|v| v.as_str().map(String::from));
        if !cat.is_some_and(|c| w.categories.contains(&c)) {
            return false;
        }
    }
    true
}
