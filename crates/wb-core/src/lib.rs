//! Núcleo de Workboard: el flujo de trabajo configurable y, en las próximas fases, el escaneo de la
//! carpeta de trabajos y la escritura del frontmatter de cada README.

pub mod error;
pub mod frontmatter;
pub mod py;
pub mod tasks;
pub mod text;
pub mod validate;
pub mod workflow;
pub mod workspace;
pub mod write;

pub use workflow::{StateCategory, Workflow, WorkflowError};
pub use workspace::Workspace;
