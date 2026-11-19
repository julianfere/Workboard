//! Trackers externos (Azure DevOps primero): leer estados, campos y work items para sincronizar, y
//! cambiar el estado de un work item solo cuando el usuario lo pide, con credenciales aparte.
//!
//! El cliente nunca guarda ni registra credenciales: las recibe al crearse y las quita de cualquier texto
//! de error. El HTTP va por un [`Transport`] para poder probarlo sin red.

pub mod azure;
pub mod html;
pub mod http;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use wb_core::StateCategory;

pub use http::{HttpRequest, HttpResponse, Transport, UreqTransport};

/// Error de un tracker: un código (clave de `locales/*.json`) y sus parámetros. Nunca lleva secretos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerError {
    pub code: &'static str,
    pub params: Vec<(&'static str, String)>,
}

impl TrackerError {
    pub fn new(code: &'static str, params: Vec<(&'static str, String)>) -> Self {
        Self { code, params }
    }
}

impl std::fmt::Display for TrackerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code)?;
        for (k, v) in &self.params {
            write!(f, " {k}={v}")?;
        }
        Ok(())
    }
}

impl std::error::Error for TrackerError {}

pub type Result<T> = std::result::Result<T, TrackerError>;

/// Un estado del proceso del tracker (agregado por nombre: cada tipo de work item tiene los suyos).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerState {
    pub name: String,
    /// Color sin `#` (como lo da el tracker), o "".
    #[serde(default)]
    pub color: String,
    /// Categoría propia del tracker (en Azure: Proposed, InProgress, Resolved, Completed, Removed).
    #[serde(default)]
    pub category: String,
    /// Tipos de work item que lo usan.
    #[serde(default)]
    pub types: Vec<String>,
    /// Categoría de estado del flujo que le corresponde (para sugerir el mapeo y saber si está cerrado).
    #[serde(default)]
    pub suggested: Option<StateCategory>,
}

/// Un campo del work item (para elegir la fecha pactada y las horas).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldInfo {
    #[serde(rename = "ref")]
    pub reference: String,
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
}

/// Qué pedir de cada work item además de lo básico. Los campos opcionales se piden solo si existen en el
/// proyecto (un campo inexistente hace fallar el pedido entero).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldRequest {
    pub priority: bool,
    pub due: Option<String>,
    pub hours: Option<String>,
    /// Trabajo completado.
    pub effort: bool,
    /// Descripción y criterios (para crear la carpeta).
    pub text: bool,
    /// Campos que se devuelven tal cual en `fields` (para elegir el campo de horas).
    pub extra: Vec<String>,
}

/// Lo leído de un work item. Es lo que queda en `tracker_cache.json` (sin `fields`, `description` ni
/// `criteria`, que solo se usan al momento).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkItem {
    pub state: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub assigned: String,
    pub changed: String,
    pub priority: Option<i64>,
    /// Fecha pactada (ISO, como la da el tracker) o "".
    pub due: String,
    /// Horas pactadas.
    pub estimate: Option<f64>,
    pub completed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criteria: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Json>,
}

impl WorkItem {
    /// Sin lo que no va a la caché.
    pub fn for_cache(mut self) -> Self {
        self.description = None;
        self.criteria = None;
        self.fields.clear();
        self
    }
}

/// Un work item para crear su carpeta: título, tipo, estado, descripción y criterios (ya en Markdown).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub state: String,
    pub description: String,
    pub criteria: String,
}

/// Estado que quedó en el tracker después de cambiarlo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateChange {
    pub state: String,
    pub changed: String,
}

/// Lo que la app necesita de un tracker. Todo es lectura salvo [`Tracker::set_state`].
pub trait Tracker: Send + Sync {
    /// Estados del proceso, ordenados por categoría.
    fn states(&self) -> Result<Vec<TrackerState>>;
    fn fields(&self) -> Result<Vec<FieldInfo>>;
    /// Work items por id; los que no existen o no se pueden ver se omiten.
    fn work_items(&self, ids: &[String], req: &FieldRequest) -> Result<BTreeMap<String, WorkItem>>;
    /// Ids asignados al dueño de la credencial con `due_field` en `[start, end)` y sin los estados
    /// `exclude`, ordenados por esa fecha. Solo lectura.
    fn assigned_to_me(
        &self,
        due_field: &str,
        start: &str,
        end: &str,
        exclude: &[String],
    ) -> Result<Vec<String>>;
    fn detail(&self, id: &str) -> Result<Detail>;
    /// ESCRIBE en el tracker: cambia el estado de un work item.
    fn set_state(&self, id: &str, state: &str) -> Result<StateChange>;
}

/// Trackers soportados.
pub const KINDS: &[&str] = &[azure::KIND];

/// Datos de conexión de un tracker (sin la credencial).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Connection {
    pub kind: String,
    pub org: String,
    pub project: String,
}

impl Connection {
    /// Valida y normaliza (URL de la organización, proyecto).
    pub fn normalized(kind: &str, org: &str, project: &str) -> Result<Self> {
        match kind {
            azure::KIND => Ok(Self {
                kind: kind.to_string(),
                org: azure::normalize_org(org)?,
                project: azure::normalize_project(project)?,
            }),
            _ => Err(TrackerError::new("tracker.unknownKind", vec![("kind", kind.to_string())])),
        }
    }

    /// Nombre para mostrar del tracker.
    pub fn source(&self) -> &'static str {
        match self.kind.as_str() {
            azure::KIND => azure::SOURCE,
            _ => "tracker",
        }
    }

    /// Página de un work item en el tracker.
    pub fn item_url(&self, id: &str) -> String {
        azure::item_url(&self.org, id)
    }

    /// Cliente con una credencial (la de lectura o la de escritura).
    pub fn client(&self, secret: &str, http: Arc<dyn Transport>) -> Result<Box<dyn Tracker>> {
        match self.kind.as_str() {
            azure::KIND => Ok(Box::new(azure::AzureDevOps::new(&self.org, &self.project, secret, http)?)),
            _ => Err(TrackerError::new("tracker.unknownKind", vec![("kind", self.kind.clone())])),
        }
    }
}
