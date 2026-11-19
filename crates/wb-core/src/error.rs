//! Errores del core: una clase (para decidir qué hacer) y un código traducible con sus parámetros
//! (para mostrarlo). El texto vive en `locales/*.json`; en español reproduce el mensaje de la v1.

use std::fmt;

/// Clase de error. Equivale a la excepción de Python que lanzaba la v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Dato inválido (ValueError).
    Invalid,
    /// No existe (FileNotFoundError).
    NotFound,
    /// Ya existe (FileExistsError).
    Exists,
    /// Fuera de la raíz (PermissionError).
    Permission,
    /// El archivo cambió mientras se escribía (RuntimeError).
    Conflict,
    /// Error del sistema de archivos.
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError {
    pub kind: ErrorKind,
    /// Clave de traducción (`core.task.empty`…).
    pub code: &'static str,
    pub params: Vec<(&'static str, String)>,
}

impl CoreError {
    pub fn new(kind: ErrorKind, code: &'static str, params: Vec<(&'static str, String)>) -> Self {
        Self { kind, code, params }
    }

    pub fn invalid(code: &'static str, params: Vec<(&'static str, String)>) -> Self {
        Self::new(ErrorKind::Invalid, code, params)
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code)?;
        for (k, v) in &self.params {
            write!(f, " {k}={v}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CoreError {}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        let kind = match e.kind() {
            std::io::ErrorKind::NotFound => ErrorKind::NotFound,
            std::io::ErrorKind::AlreadyExists => ErrorKind::Exists,
            _ => ErrorKind::Io,
        };
        Self::new(kind, "core.io", vec![("message", e.to_string())])
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
