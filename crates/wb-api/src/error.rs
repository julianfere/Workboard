//! Errores que cruzan a la UI: un código (clave de `locales/*.json`) y sus parámetros. La UI los traduce;
//! el backend nunca arma el texto. Equivale al `safe()` de la v1: ningún comando deja escapar un panic
//! ni un error crudo.

use std::collections::BTreeMap;

use serde::Serialize;
use wb_core::WorkflowError;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApiError {
    pub code: String,
    pub params: BTreeMap<String, String>,
}

impl ApiError {
    pub fn new(code: &str, params: &[(&str, &str)]) -> Self {
        Self {
            code: code.to_string(),
            params: params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    /// Error inesperado: el mensaje va como parámetro de `error.generic`.
    pub fn generic(message: impl ToString) -> Self {
        Self::new("error.generic", &[("message", &message.to_string())])
    }
}

impl From<WorkflowError> for ApiError {
    fn from(e: WorkflowError) -> Self {
        match e {
            WorkflowError::UnknownPreset(id) => Self::new("error.unknownPreset", &[("id", &id)]),
            WorkflowError::Json(err) => Self::new("error.invalidWorkflow", &[("message", &err.to_string())]),
            WorkflowError::Invalid(msg) => Self::new("error.invalidWorkflow", &[("message", &msg)]),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::generic(e)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_errors_map_to_translatable_codes() {
        let e: ApiError = WorkflowError::UnknownPreset("x".into()).into();
        assert_eq!(e, ApiError::new("error.unknownPreset", &[("id", "x")]));
        // Cada código que puede salir existe en los catálogos.
        for code in ["error.unknownPreset", "error.invalidWorkflow", "error.generic"] {
            assert_ne!(wb_i18n::t("en", code, &[]), code, "{code} missing from locales");
        }
    }
}
