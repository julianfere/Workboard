//! Núcleo de Workboard: el flujo de trabajo configurable y, en las próximas fases, el escaneo de la
//! carpeta de trabajos y la escritura del frontmatter de cada README.

pub mod workflow;

pub use workflow::{StateCategory, Workflow, WorkflowError};
