//! Preferencias de la app (`<config>/settings.json`): idioma, tema, preset de flujo, carpeta abierta,
//! recientes y carpeta de notas. El MCP lee el mismo archivo. Un archivo que no existe o no se puede leer
//! equivale a los valores por defecto; la escritura es atómica.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_RECENTS: usize = 12;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// `None` = el idioma del sistema.
    pub lang: Option<String>,
    pub theme: Theme,
    /// Id del preset de flujo.
    pub workflow: String,
    /// Carpeta de trabajos abierta.
    pub root: Option<String>,
    /// Carpetas abiertas antes, la más reciente primero.
    pub recents: Vec<String>,
    /// Carpeta de notas (`None` = `<config>/notes`).
    pub notes_folder: Option<String>,
    /// Agentes de las sesiones: el de por defecto, el comando propio y prefijos de planificación.
    pub agents: wb_sessions::AgentSettings,
    /// Panel Git: repos de cada carpeta de trabajos y ramas candidatas de cada ambiente.
    pub git: GitSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GitSettings {
    /// Repos que se revisan en cada carpeta de trabajos (clave: la carpeta). Sin entrada = los repos
    /// hermanos detectados.
    pub repos: BTreeMap<String, Vec<wb_git::RepoConfig>>,
    /// Ramas remotas candidatas de cada ambiente para detectar repos (`None` = las del flujo).
    pub env_refs: Option<BTreeMap<String, Vec<String>>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: None,
            theme: Theme::System,
            workflow: "default".into(),
            root: None,
            recents: vec![],
            notes_folder: None,
            agents: Default::default(),
            git: Default::default(),
        }
    }
}

impl Settings {
    /// Deja `root` como carpeta abierta y primera de los recientes.
    pub fn open_root(&mut self, root: &str) {
        self.recents.retain(|r| !r.eq_ignore_ascii_case(root));
        self.recents.insert(0, root.to_string());
        self.recents.truncate(MAX_RECENTS);
        self.root = Some(root.to_string());
    }
}

pub fn path_in(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}

pub fn load(path: &Path) -> Settings {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wb-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_or_broken_file_gives_defaults() {
        let dir = temp_dir("broken");
        let path = path_in(&dir);
        assert_eq!(load(&path), Settings::default());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path), Settings::default());
        std::fs::write(&path, r#"{"theme": "dark", "unknown": 1}"#).unwrap();
        assert_eq!(load(&path), Settings { theme: Theme::Dark, ..Settings::default() });
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn roundtrip_and_recents() {
        let dir = temp_dir("roundtrip");
        let path = path_in(&dir);
        let mut s = Settings {
            lang: Some("es".into()),
            theme: Theme::Light,
            workflow: "classic-es".into(),
            ..Default::default()
        };
        s.open_root("C:/a");
        s.open_root("C:/b");
        s.open_root("c:/A");
        assert_eq!(s.recents, ["c:/A", "C:/b"]);
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
