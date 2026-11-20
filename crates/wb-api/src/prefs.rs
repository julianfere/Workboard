//! Preferencias de la UI (`<config>/ui_prefs.json`): un objeto `{clave: valor}` opaco para el backend
//! (columnas y vistas guardadas de la grilla, grupos plegados…). Cada clave se escribe sola, así dos vistas
//! que guardan cosas distintas no se pisan.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value as Json};

use crate::{Api, ApiError, ApiResult, Args};

/// Tope de lo que se guarda por clave (la UI guarda listas chicas; esto evita llenar el disco por un error).
const MAX_VALUE_BYTES: usize = 512 * 1024;

fn path_in(config_dir: &Path) -> PathBuf {
    config_dir.join("ui_prefs.json")
}

fn load(path: &Path) -> Map<String, Json> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Json>(&s).ok())
        .and_then(|v| match v {
            Json::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

fn save(path: &Path, prefs: &Map<String, Json>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(prefs).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, path)
}

impl Api {
    pub(crate) fn prefs_command(&self, cmd: &str, a: &Args) -> Option<ApiResult<Json>> {
        let out = match cmd {
            "ui_prefs" => Ok(Json::Object(load(&path_in(&self.config_dir)))),
            "ui_pref_set" => self.set_pref(a),
            _ => return None,
        };
        Some(out)
    }

    /// Escribe una clave (`null` la borra) y devuelve todas.
    fn set_pref(&self, a: &Args) -> ApiResult<Json> {
        let key = a.str("key")?.trim();
        if key.is_empty() || key.len() > 200 {
            return Err(ApiError::new("error.badArgument", &[("name", "key")]));
        }
        let value = a.get("value").clone();
        if value.to_string().len() > MAX_VALUE_BYTES {
            return Err(ApiError::new("error.badArgument", &[("name", "value")]));
        }
        // Las escrituras de la UI llegan de a una, pero el lock evita que dos comandos simultáneos se pisen.
        let _guard = self.prefs_lock.lock().expect("prefs lock");
        let path = path_in(&self.config_dir);
        let mut prefs = load(&path);
        if value.is_null() {
            prefs.remove(key);
        } else {
            prefs.insert(key.to_string(), value);
        }
        save(&path, &prefs)?;
        Ok(Json::Object(prefs))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Api;

    #[test]
    fn set_read_and_remove() {
        let dir = std::env::temp_dir().join(format!("wb-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let api = Api::new(dir.clone());
        assert_eq!(api.dispatch("ui_prefs", &json!({})).unwrap(), json!({}));
        api.dispatch("ui_pref_set", &json!({"key": "grid", "value": {"views": [1, 2]}})).unwrap();
        api.dispatch("ui_pref_set", &json!({"key": "other", "value": true})).unwrap();
        let all = api.dispatch("ui_prefs", &json!({})).unwrap();
        assert_eq!(all["grid"]["views"], json!([1, 2]));
        assert_eq!(all["other"], json!(true));
        let after = api.dispatch("ui_pref_set", &json!({"key": "other", "value": null})).unwrap();
        assert!(after.get("other").is_none());
        assert_eq!(
            Api::new(dir.clone()).dispatch("ui_prefs", &json!({})).unwrap()["grid"]["views"],
            json!([1, 2])
        );
        let bad = api.dispatch("ui_pref_set", &json!({"key": " ", "value": 1})).unwrap_err();
        assert_eq!(bad.code, "error.badArgument");
        // Un archivo roto no rompe nada: vuelve a empezar vacío.
        std::fs::write(dir.join("ui_prefs.json"), "{nope").unwrap();
        assert_eq!(api.dispatch("ui_prefs", &json!({})).unwrap(), json!({}));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
