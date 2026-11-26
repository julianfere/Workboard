//! Preferencias de la app (`<config>/settings.json`): idioma, tema, preset de flujo, carpeta abierta,
//! recientes (con su nombre para mostrar) y carpeta de notas. El MCP lee el mismo archivo. Un archivo que no existe o no se puede leer
//! equivale a los valores por defecto; la escritura es atómica.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_RECENTS: usize = 12;
/// Largo máximo de un nombre para mostrar (como la v1).
pub const MAX_ALIAS_LEN: usize = 60;

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
    /// Nombre para mostrar de una carpeta de trabajos (clave: la ruta como en `recents`; se compara sin
    /// distinguir mayúsculas). Solo en la app: la carpeta en disco no cambia.
    pub aliases: BTreeMap<String, String>,
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
            aliases: BTreeMap::new(),
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

    /// Saca `path` de los recientes; devuelve en qué posición estaba (para deshacer).
    pub fn forget_recent(&mut self, path: &str) -> Option<usize> {
        let pos = self.recents.iter().position(|r| r.eq_ignore_ascii_case(path))?;
        self.recents.remove(pos);
        Some(pos)
    }

    /// Vuelve a poner `path` en los recientes en la posición `pos` (deshacer «Quitar de recientes»).
    pub fn restore_recent(&mut self, path: &str, pos: usize) {
        self.recents.retain(|r| !r.eq_ignore_ascii_case(path));
        self.recents.insert(pos.min(self.recents.len()), path.to_string());
        self.recents.truncate(MAX_RECENTS);
    }

    /// Clave de `path` en `aliases` (la que ya existe, sin distinguir mayúsculas).
    fn alias_key(&self, path: &str) -> Option<&String> {
        self.aliases.keys().find(|k| k.eq_ignore_ascii_case(path))
    }

    /// Nombre para mostrar de una carpeta, si tiene uno.
    pub fn alias_of(&self, path: &str) -> Option<&str> {
        self.alias_key(path).and_then(|k| self.aliases.get(k)).map(String::as_str)
    }

    /// Pone (o con un nombre vacío, quita) el nombre para mostrar de `path`. Los espacios se normalizan.
    /// `Err(máximo)` si el nombre es demasiado largo.
    pub fn set_alias(&mut self, path: &str, name: &str) -> Result<(), usize> {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.chars().count() > MAX_ALIAS_LEN {
            return Err(MAX_ALIAS_LEN);
        }
        if let Some(k) = self.alias_key(path).cloned() {
            self.aliases.remove(&k);
        }
        if !name.is_empty() {
            self.aliases.insert(path.to_string(), name);
        }
        Ok(())
    }

    /// Nombre para mostrar: el alias o, si no hay, el de la carpeta; una carpeta de trabajos tipo `_work`
    /// (nombre que empieza con `_` o `.`) toma el de la carpeta que la contiene
    /// (`…/ContractHub/_work` → `ContractHub`), como la v1.
    pub fn display_name(&self, path: &str) -> String {
        self.alias_of(path).map_or_else(|| folder_display_name(path), String::from)
    }
}

/// Prefijos de una carpeta de trabajos "interna" de un proyecto (`_work`, `.work`).
const INNER_ROOT_PREFIXES: [char; 2] = ['_', '.'];

/// Nombre de una carpeta de trabajos sin alias (ver `Settings::display_name`).
pub fn folder_display_name(path: &str) -> String {
    let p = Path::new(path);
    let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return path.to_string();
    };
    let parent = p.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
    match parent {
        Some(parent) if name.starts_with(INNER_ROOT_PREFIXES) && !parent.is_empty() => parent,
        _ => name,
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

    #[test]
    fn old_settings_without_aliases_still_load() {
        let dir = temp_dir("old");
        let path = path_in(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"lang": "es", "theme": "dark", "workflow": "classic-es", "root": "C:/a", "recents": ["C:/a"]}"#)
            .unwrap();
        let s = load(&path);
        assert_eq!(s.recents, ["C:/a"]);
        assert_eq!(s.theme, Theme::Dark);
        assert!(s.aliases.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn claude_plans_with_go_by_default() {
        let s = Settings::default();
        assert_eq!(s.agents.plan_prefix.get("claude").map(String::as_str), Some("/go "));
        assert_eq!(s.agents.plan_prefix.len(), 1);
        let s: Settings = serde_json::from_str(r#"{"agents": {"default": "copilot"}}"#).unwrap();
        assert_eq!(s.agents.plan_prefix.get("claude").map(String::as_str), Some("/go "));
        // Vaciado a propósito por el usuario: se respeta.
        let s: Settings = serde_json::from_str(r#"{"agents": {"planPrefix": {}}}"#).unwrap();
        assert!(s.agents.plan_prefix.is_empty());
    }

    #[test]
    fn aliases_and_display_names() {
        let mut s = Settings::default();
        // Una carpeta tipo `_work` toma el nombre del proyecto que la contiene (v1).
        assert_eq!(s.display_name("C:/x/_work"), "x");
        assert_eq!(s.display_name("C:/x/.work"), "x");
        assert_eq!(s.display_name("C:/x/trabajos"), "trabajos");
        assert_eq!(s.display_name("_work"), "_work");
        s.set_alias("C:/x/_work", "  Contratos   (back) ").unwrap();
        assert_eq!(s.alias_of("c:/X/_WORK"), Some("Contratos (back)"));
        assert_eq!(s.display_name("C:/x/_work"), "Contratos (back)");
        // Otra grafía de la misma carpeta reemplaza, no duplica.
        s.set_alias("c:/x/_work", "Otro").unwrap();
        assert_eq!(s.aliases.len(), 1);
        assert_eq!(s.alias_of("C:/x/_work"), Some("Otro"));
        assert_eq!(s.set_alias("C:/x/_work", &"a".repeat(61)), Err(MAX_ALIAS_LEN));
        s.set_alias("C:/x/_work", "").unwrap();
        assert!(s.aliases.is_empty());
        // Ida y vuelta por JSON.
        s.set_alias("C:/y", "Y").unwrap();
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn forget_and_restore_recent() {
        let mut s = Settings::default();
        s.open_root("C:/a");
        s.open_root("C:/b");
        s.open_root("C:/c");
        assert_eq!(s.forget_recent("c:/B"), Some(1));
        assert_eq!(s.recents, ["C:/c", "C:/a"]);
        assert_eq!(s.forget_recent("C:/zzz"), None);
        s.restore_recent("C:/b", 1);
        assert_eq!(s.recents, ["C:/c", "C:/b", "C:/a"]);
        s.restore_recent("C:/z", 99);
        assert_eq!(s.recents.last().unwrap(), "C:/z");
    }
}
