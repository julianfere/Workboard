//! Flujo de trabajo configurable: estados, tipos, ambientes, período, convenciones de carpetas, claves
//! del frontmatter y categorías de archivo. Todo lo que en la v1 eran constantes de `core.py` sale de un
//! preset (`presets/*.json`); `classic-es` reproduce la v1 exactamente.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Textos por idioma (`{"en": "...", "es": "..."}`).
pub type Labels = BTreeMap<String, String>;

const PRESETS: &[(&str, &str)] = &[
    ("default", include_str!("../presets/default.json")),
    ("classic-es", include_str!("../presets/classic-es.json")),
];

#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    #[error("unknown workflow preset: {0}")]
    UnknownPreset(String),
    #[error("invalid workflow JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid workflow: {0}")]
    Invalid(String),
}

/// Para qué sirve cada estado, independiente de su nombre. Ordena la UI, decide qué está cerrado y
/// permite mapear estados de otras fuentes (Azure, GitHub…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StateCategory {
    Backlog,
    Planned,
    Active,
    Review,
    Done,
    Cancelled,
    Unknown,
}

impl StateCategory {
    pub fn is_closed(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateDef {
    pub id: String,
    pub category: StateCategory,
    pub labels: Labels,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Choice {
    pub id: String,
    pub labels: Labels,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeriodRule {
    /// Día en que empieza cada período (1 = mes calendario; la v1 usa 20: del 20 al 19).
    pub start_day: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FolderRules {
    /// Regex sobre el nombre de la carpeta con los grupos `id` (y opcional `id2`) del item.
    pub item_pattern: String,
    /// Regex que tiene que cumplir cada id cargado a mano en el frontmatter.
    pub item_id_pattern: String,
    /// Nombre sugerido para la carpeta de un item nuevo: `{id}` y `{slug}` (del título).
    pub new_item_template: String,
    pub group_prefix: String,
    pub deleted_dir: String,
    /// Carpetas que no se escanean.
    pub ignore_prefixes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtraField {
    pub key: String,
    #[serde(default)]
    pub no_spaces: bool,
}

/// Claves del frontmatter de cada dato que la app entiende.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldKeys {
    pub status: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub environments: String,
    pub item_ids: String,
    pub notes: String,
    pub branch: String,
    pub title: String,
    pub due: String,
    pub period: String,
    pub hours: String,
    /// Sesiones de agentes que trabajaron el item (lista de ids; la escribe la app, no se edita a mano).
    pub sessions: String,
    /// Campos de texto de una línea que agrega el flujo (o un plugin).
    #[serde(default)]
    pub extra: Vec<ExtraField>,
}

impl FieldKeys {
    /// Claves editables, en el orden de la v1 (`core.EDITABLE`).
    pub fn editable(&self) -> Vec<&str> {
        let mut out = vec![
            self.status.as_str(),
            &self.kind,
            &self.environments,
            &self.item_ids,
            &self.due,
            &self.period,
            &self.hours,
            &self.notes,
            &self.branch,
            &self.title,
        ];
        out.extend(self.extra.iter().map(|f| f.key.as_str()));
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRules {
    /// Título de la sección de tareas (`## Pendientes`); se reconoce sin distinguir mayúsculas.
    pub heading: String,
    pub max_len: usize,
}

/// Claves del frontmatter de las notas personales y el título de una nota sin título.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteRules {
    pub items_key: String,
    pub author_key: String,
    pub created_key: String,
    pub untitled: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanRules {
    pub heading: String,
    pub placeholder: String,
    pub min_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileCategory {
    pub id: String,
    /// Se puede marcar como corrido en un ambiente.
    pub runnable: bool,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    pub labels: Labels,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileRules {
    /// Tipos de archivo que se detectan por una palabra del nombre (`MIGRATION_x.sql`), en orden.
    pub kinds: Vec<String>,
    /// Gana la primera categoría cuyo tipo o extensión coincide.
    pub categories: Vec<FileCategory>,
    pub fallback_category: FileCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Workflow {
    pub id: String,
    pub name: Labels,
    pub states: Vec<StateDef>,
    pub types: Vec<Choice>,
    pub environments: Vec<Choice>,
    pub period: PeriodRule,
    pub folders: FolderRules,
    pub fields: FieldKeys,
    pub tasks: TaskRules,
    /// Palabras (en minúsculas) que identifican la sección "cómo deployar" del README.
    #[serde(default)]
    pub deploy_keywords: Vec<String>,
    pub notes: NoteRules,
    /// Título del archivo de la bandeja de tareas sin item.
    pub inbox_title: String,
    pub plan: PlanRules,
    pub files: FileRules,
    #[serde(skip)]
    compiled: OnceLock<Compiled>,
}

#[derive(Debug, Clone)]
struct Compiled {
    item: Regex,
    item_id: Regex,
    kinds: Vec<(String, Regex)>,
}

impl Workflow {
    /// Ids de los presets incluidos.
    pub fn preset_ids() -> impl Iterator<Item = &'static str> {
        PRESETS.iter().map(|(id, _)| *id)
    }

    pub fn preset(id: &str) -> Result<Self, WorkflowError> {
        let (_, json) =
            PRESETS.iter().find(|(p, _)| *p == id).ok_or_else(|| WorkflowError::UnknownPreset(id.into()))?;
        Self::from_json(json)
    }

    pub fn from_json(json: &str) -> Result<Self, WorkflowError> {
        let wf: Self = serde_json::from_str(json)?;
        wf.validate()?;
        Ok(wf)
    }

    fn validate(&self) -> Result<(), WorkflowError> {
        let bad = |msg: String| Err(WorkflowError::Invalid(format!("{}: {msg}", self.id)));
        if self.states.is_empty() {
            return bad("at least one state is required".into());
        }
        for (what, ids) in [
            ("state", self.states.iter().map(|s| s.id.as_str()).collect::<Vec<_>>()),
            ("type", self.types.iter().map(|t| t.id.as_str()).collect()),
            ("environment", self.environments.iter().map(|e| e.id.as_str()).collect()),
            ("file category", self.categories().map(|c| c.id.as_str()).collect()),
        ] {
            let mut seen = HashSet::new();
            for id in ids {
                if id.is_empty() || !seen.insert(id) {
                    return bad(format!("empty or repeated {what} id {id:?}"));
                }
            }
        }
        if !(1..=28).contains(&self.period.start_day) {
            return bad("period.startDay must be between 1 and 28".into());
        }
        let keys = self.fields.editable();
        if keys.iter().collect::<HashSet<_>>().len() != keys.len() {
            return bad("repeated frontmatter key".into());
        }
        if let Some(k) =
            keys.iter().find(|k| k.is_empty() || !k.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
        {
            return bad(format!("frontmatter keys use only a-z and _: {k:?}"));
        }
        for c in &self.files.categories {
            if let Some(k) = c.kinds.iter().find(|k| !self.files.kinds.contains(k)) {
                return bad(format!("category {} uses unknown kind {k}", c.id));
            }
        }
        let compiled = self.compile().map_err(|e| WorkflowError::Invalid(format!("{}: {e}", self.id)))?;
        let _ = self.compiled.set(compiled);
        Ok(())
    }

    fn compile(&self) -> Result<Compiled, String> {
        let item = Regex::new(&self.folders.item_pattern).map_err(|e| format!("folders.itemPattern: {e}"))?;
        if !item.capture_names().any(|n| n == Some("id")) {
            return Err("folders.itemPattern needs a named group `id`".into());
        }
        let item_id =
            Regex::new(&self.folders.item_id_pattern).map_err(|e| format!("folders.itemIdPattern: {e}"))?;
        let kinds = self
            .files
            .kinds
            .iter()
            .map(|k| {
                // Igual que la v1: la palabra no puede estar pegada a otras letras (MIGRATIONS no es MIGRATION).
                let re = Regex::new(&format!("(^|[^A-Z]){}([^A-Z]|$)", regex::escape(k))).expect("escaped");
                (k.clone(), re)
            })
            .collect();
        Ok(Compiled { item, item_id, kinds })
    }

    fn compiled(&self) -> &Compiled {
        // Lo normal es que ya esté compilado (from_json valida). Un Workflow deserializado por otro
        // camino se compila acá; si sus regex no compilan es un error de programación.
        self.compiled.get_or_init(|| self.compile().expect("workflow must be built with Workflow::from_json"))
    }

    // ---------- estados ----------

    pub fn state(&self, id: &str) -> Option<&StateDef> {
        self.states.iter().find(|s| s.id == id)
    }

    pub fn state_ids(&self) -> impl Iterator<Item = &str> {
        self.states.iter().map(|s| s.id.as_str())
    }

    /// Posición del estado en el flujo (para ordenar y detectar retrocesos).
    pub fn state_rank(&self, id: &str) -> Option<usize> {
        self.states.iter().position(|s| s.id == id)
    }

    pub fn is_closed(&self, id: &str) -> bool {
        self.state(id).is_some_and(|s| s.category.is_closed())
    }

    pub fn closed_states(&self) -> impl Iterator<Item = &str> {
        self.states.iter().filter(|s| s.category.is_closed()).map(|s| s.id.as_str())
    }

    // ---------- carpetas ----------

    /// Ids de item que lleva el nombre de una carpeta (`wi123456-654321-x` → 123456, 654321).
    pub fn item_ids_in_folder(&self, folder: &str) -> Vec<String> {
        let Some(c) = self.compiled().item.captures(folder) else { return vec![] };
        ["id", "id2"].iter().filter_map(|g| c.name(g)).map(|m| m.as_str().to_string()).collect()
    }

    pub fn is_valid_item_id(&self, id: &str) -> bool {
        self.compiled().item_id.is_match(id)
    }

    pub fn is_ignored_folder(&self, name: &str) -> bool {
        self.folders.ignore_prefixes.iter().any(|p| name.starts_with(p.as_str()))
    }

    pub fn is_group_folder(&self, name: &str) -> bool {
        !self.is_ignored_folder(name)
            && name.to_lowercase().starts_with(&self.folders.group_prefix.to_lowercase())
    }

    // ---------- período ----------

    /// Clave `AAAA-MM` del período en que cae la fecha: el que empieza el `startDay` de ese mes, o del
    /// anterior si la fecha es antes de ese día.
    pub fn period_key(&self, year: i32, month: u32, day: u32) -> String {
        let (mut y, mut m) = (year, month);
        if day < u32::from(self.period.start_day) {
            if m == 1 {
                y -= 1;
                m = 12;
            } else {
                m -= 1;
            }
        }
        format!("{y:04}-{m:02}")
    }

    // ---------- archivos ----------

    /// Tipo del archivo por una palabra de su nombre (`DIAG_x.sql` → DIAG).
    pub fn file_kind(&self, name: &str) -> Option<&str> {
        let upper = name.to_uppercase();
        self.compiled().kinds.iter().find(|(_, re)| re.is_match(&upper)).map(|(k, _)| k.as_str())
    }

    pub fn categories(&self) -> impl Iterator<Item = &FileCategory> {
        self.files.categories.iter().chain(std::iter::once(&self.files.fallback_category))
    }

    pub fn file_category(&self, name: &str) -> &FileCategory {
        let ext = Path::new(name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        let kind = self.file_kind(name);
        self.files
            .categories
            .iter()
            .find(|c| {
                kind.is_some_and(|k| c.kinds.iter().any(|ck| ck == k))
                    || (!ext.is_empty() && c.extensions.contains(&ext))
            })
            .unwrap_or(&self.files.fallback_category)
    }

    // ---------- textos ----------

    /// Texto en `lang`, o en inglés, o el id.
    pub fn label<'a>(labels: &'a Labels, lang: &str, id: &'a str) -> &'a str {
        labels.get(lang).or_else(|| labels.get("en")).map_or(id, String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_load_and_validate() {
        for id in Workflow::preset_ids() {
            let wf = Workflow::preset(id).unwrap();
            assert_eq!(wf.id, id);
        }
        assert!(matches!(Workflow::preset("nope"), Err(WorkflowError::UnknownPreset(_))));
    }

    #[test]
    fn rejects_bad_definitions() {
        let base: serde_json::Value = serde_json::from_str(PRESETS[0].1).unwrap();
        let broken = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut v = base.clone();
            f(&mut v);
            Workflow::from_json(&v.to_string())
        };
        assert!(broken(&|v| v["states"] = serde_json::json!([])).is_err());
        assert!(broken(&|v| v["states"][1]["id"] = v["states"][0]["id"].clone()).is_err());
        assert!(broken(&|v| v["period"]["startDay"] = 0.into()).is_err());
        assert!(broken(&|v| v["folders"]["itemPattern"] = "^(\\d+)".into()).is_err());
        assert!(broken(&|v| v["folders"]["itemPattern"] = "(".into()).is_err());
        assert!(broken(&|v| v["fields"]["notes"] = "Notes".into()).is_err());
        assert!(broken(&|v| v["fields"]["notes"] = v["fields"]["status"].clone()).is_err());
        assert!(broken(&|v| v["files"]["categories"][1]["kinds"] = serde_json::json!(["NOPE"])).is_err());
        assert!(broken(&|v| v["extra"] = 1.into()).is_err(), "unknown fields are rejected");
    }

    #[test]
    fn default_preset_basics() {
        let wf = Workflow::preset("default").unwrap();
        assert_eq!(wf.item_ids_in_folder("PROJ-123-login"), ["PROJ-123"]);
        assert_eq!(wf.item_ids_in_folder("42-fix-typo"), ["42"]);
        assert!(wf.item_ids_in_folder("login").is_empty());
        assert_eq!(wf.period_key(2026, 1, 1), "2026-01");
        assert_eq!(wf.closed_states().collect::<Vec<_>>(), ["done", "cancelled"]);
        assert_eq!(wf.file_category("MIGRATION_add.sql").id, "migration");
        assert_eq!(Workflow::label(&wf.states[0].labels, "es", "backlog"), "Pendiente");
        assert_eq!(Workflow::label(&wf.states[0].labels, "fr", "backlog"), "Backlog");
    }

    #[test]
    fn clones_keep_working() {
        let wf = Workflow::preset("classic-es").unwrap().clone();
        assert_eq!(wf.item_ids_in_folder("wi123456-x"), ["123456"]);
    }
}
