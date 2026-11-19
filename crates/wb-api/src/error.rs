//! Errores que cruzan a la UI: un código (clave de `locales/*.json`) y sus parámetros. La UI los traduce;
//! el backend nunca arma el texto. Equivale al `safe()` de la v1: ningún comando deja escapar un error
//! crudo.

use std::collections::BTreeMap;

use serde::Serialize;
use wb_core::WorkflowError;
use wb_core::error::{CoreError, ErrorKind};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApiError {
    pub code: String,
    pub params: BTreeMap<String, String>,
    /// El README cambió en disco mientras se editaba: la UI ofrece reaplicar o descartar.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
}

impl ApiError {
    pub fn new(code: &str, params: &[(&str, &str)]) -> Self {
        Self {
            code: code.to_string(),
            params: params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            conflict: false,
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

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        let params: Vec<(&str, &str)> = e.params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mut out = Self::new(e.code, &params);
        out.conflict = e.kind == ErrorKind::Conflict || e.code == "core.task.changed";
        out
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
    fn errors_map_to_translatable_codes() {
        let e: ApiError = WorkflowError::UnknownPreset("x".into()).into();
        assert_eq!(e, ApiError::new("error.unknownPreset", &[("id", "x")]));
        let c: ApiError = CoreError::new(ErrorKind::Conflict, "core.write.conflict", vec![]).into();
        assert!(c.conflict);
        for code in ["error.unknownPreset", "error.invalidWorkflow", "error.generic", "core.write.conflict"] {
            assert_ne!(wb_i18n::t("en", code, &[]), code, "{code} missing from locales");
        }
    }
}
