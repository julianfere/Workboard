//! Registro de errores de la API (`<config>/workboard.log`, como el de la v1): una línea por error con la
//! fecha, el comando y el código con sus parámetros. Nunca los argumentos del comando (pueden traer
//! tokens); los mensajes de error ya vienen sin secretos. Pasado 1 MB rota a `workboard.log.1`.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::ApiError;

pub const LOG_FILE: &str = "workboard.log";
pub const LOG_MAX_BYTES: u64 = 1_000_000;

pub fn path_in(config_dir: &Path) -> PathBuf {
    config_dir.join(LOG_FILE)
}

/// Agrega el error al registro. Si no se puede escribir, se ignora (el error ya vuelve a la UI).
pub fn write(config_dir: &Path, cmd: &str, err: &ApiError) {
    let _ = try_write(config_dir, cmd, err, LOG_MAX_BYTES);
}

fn try_write(config_dir: &Path, cmd: &str, err: &ApiError, max: u64) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let path = path_in(config_dir);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > max) {
        std::fs::rename(&path, config_dir.join(format!("{LOG_FILE}.1")))?;
    }
    let params = serde_json::to_string(&err.params).unwrap_or_default();
    let line = format!(
        "{} {cmd}: {} {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        err.code,
        params.replace(['\r', '\n'], " ")
    );
    std::fs::OpenOptions::new().create(true).append(true).open(&path)?.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Api;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-api-errlog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn api_errors_go_to_the_log_without_the_arguments() {
        let dir = temp("api");
        let api = Api::new(dir.clone());
        api.dispatch("tracker_connect", &json!({"kind": "nope", "secret": "s3cr3t-token"})).unwrap_err();
        api.dispatch("no_such_command", &json!({"secret": "s3cr3t-token"})).unwrap_err();
        api.dispatch("app_info", &json!({})).unwrap();
        let log = std::fs::read_to_string(path_in(&dir)).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2, "{log}");
        assert!(
            lines[1].contains("no_such_command: error.unknownCommand {\"name\":\"no_such_command\"}"),
            "{log}"
        );
        assert!(!log.contains("s3cr3t"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_log_rotates_past_the_limit() {
        let dir = temp("rotate");
        let err = ApiError::new("error.generic", &[("message", "boom\nline")]);
        try_write(&dir, "a", &err, 100).unwrap();
        assert!(!std::fs::read_to_string(path_in(&dir)).unwrap().contains("boom\n"));
        for _ in 0..5 {
            try_write(&dir, "b", &err, 100).unwrap();
        }
        let old = std::fs::read_to_string(dir.join("workboard.log.1")).unwrap();
        let cur = std::fs::read_to_string(path_in(&dir)).unwrap();
        assert!(old.len() as u64 > 100 && cur.len() <= old.len(), "{old}\n---\n{cur}");
        assert!(old.starts_with(&chrono::Local::now().format("%Y-").to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
