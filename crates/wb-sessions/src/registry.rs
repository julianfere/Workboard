//! Registro de sesiones de una raíz: `<config>/sessions/<carpeta>-<hash>.json` = `{"sessions": [...]}`.
//! Lo escribe solo la app; el MCP lo lee en cada llamada para saber qué items puede escribir cada sesión.
//! El estado en vivo (trabajando, te necesita…) llega por hooks y no se guarda acá.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Con proceso (o recién lanzada).
    #[default]
    Active,
    /// Sin proceso: se retoma con Reanudar.
    Paused,
    /// Fuera de la lista principal; se puede reabrir.
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Session {
    pub id: String,
    pub name: String,
    /// Carpeta de trabajos.
    pub root: String,
    /// Donde corre el agente: el padre de la carpeta de trabajos (se mueve solo entre los repos).
    pub cwd: String,
    /// Items (carpetas relativas a la raíz) que la sesión puede escribir.
    pub items: Vec<String>,
    pub state: State,
    /// Modo de permisos: `default`, `plan` o `acceptEdits`.
    pub mode: String,
    pub agent: String,
    /// Sesión de planificación (el hook `Stop` valida el plan).
    pub plan: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Hora local `AAAA-MM-DDTHH:MM:SS`, como la v1.
    pub created: String,
    pub last_activity: String,
    /// Sesión de la que heredó la conversación (Separar).
    pub fork_of: Option<String>,
    /// Repos (primera carpeta bajo `cwd`) donde editó archivos.
    pub repos: Vec<String>,
    /// Fijada arriba en la barra lateral.
    pub pinned: bool,
    /// Reabierta a mano: ya no se cierra sola cuando sus items terminan.
    pub keep_open: bool,
}

/// Hora local para el registro.
pub fn now() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// Epoch de una hora del registro (local) o del transcript (ISO con zona); 0 si no se entiende.
pub fn epoch(ts: &str) -> f64 {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(ts) {
        return d.timestamp_millis() as f64 / 1000.0;
    }
    chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .and_then(|n| n.and_local_timezone(chrono::Local).earliest())
        .map_or(0.0, |d| d.timestamp() as f64)
}

/// Clave de una raíz para el nombre del archivo: `<padre>-<carpeta>-<sha1[:8]>` (como la v1).
pub fn root_key(root: &Path) -> String {
    let canon = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let root = canon.as_path();
    let name = |p: Option<&Path>| {
        p.and_then(Path::file_name).map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    };
    let norm = normcase(root);
    let hash = sha1_smol::Sha1::from(norm.as_bytes()).digest().to_string();
    format!("{}-{}-{}", name(root.parent()), name(Some(root)), &hash[..8])
}

/// Ruta comparable: canónica y, en Windows, con `\` y en minúsculas (`os.path.normcase`).
pub fn normcase(p: &Path) -> String {
    let s = dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).display().to_string();
    if cfg!(windows) { s.replace('/', "\\").to_lowercase() } else { s }
}

static LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = LazyLock::new(Default::default);

#[derive(Clone)]
pub struct Registry {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl Registry {
    pub fn new(path: PathBuf) -> Self {
        let lock = LOCKS.lock().expect("registry locks").entry(path.clone()).or_default().clone();
        Self { path, lock }
    }

    pub fn of_root(config_dir: &Path, root: &Path) -> Self {
        Self::new(config_dir.join("sessions").join(format!("{}.json", root_key(root))))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Vec<Session> {
        let Ok(text) = std::fs::read_to_string(&self.path) else { return vec![] };
        let data: Json = serde_json::from_str(&text).unwrap_or(Json::Null);
        let list = if data.is_array() { data } else { data["sessions"].clone() };
        serde_json::from_value(list).unwrap_or_default()
    }

    fn save(&self, sessions: &[Session]) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text =
            serde_json::to_string_pretty(&json!({ "sessions": sessions })).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.path)
    }

    /// Lee, aplica `f` y guarda, todo con el lock del archivo.
    pub fn update<T>(&self, f: impl FnOnce(&mut Vec<Session>) -> T) -> std::io::Result<T> {
        let _guard = self.lock.lock().expect("registry lock");
        let mut sessions = self.load();
        let out = f(&mut sessions);
        self.save(&sessions)?;
        Ok(out)
    }

    /// Cambia una sesión (si existe) y devuelve cómo quedó.
    pub fn modify(&self, id: &str, f: impl FnOnce(&mut Session)) -> std::io::Result<Option<Session>> {
        self.update(|ss| {
            ss.iter_mut().find(|s| s.id == id).map(|s| {
                f(s);
                s.clone()
            })
        })
    }

    pub fn get(&self, id: &str) -> Option<Session> {
        self.load().into_iter().find(|s| s.id == id)
    }

    /// Registro de la raíz que tiene la sesión `id` (las sesiones de otros proyectos siguen vivas).
    pub fn find(config_dir: &Path, id: &str) -> Option<Self> {
        let dir = std::fs::read_dir(config_dir.join("sessions")).ok()?;
        dir.flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .map(Self::new)
            .find(|r| r.load().iter().any(|s| s.id == id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_matches_the_v1_shape() {
        let k = root_key(Path::new("C:/Proyectos/app/_work"));
        assert!(k.starts_with("app-_work-"), "{k}");
        assert_eq!(k.len(), "app-_work-".len() + 8);
    }

    #[test]
    fn epoch_reads_local_and_utc() {
        assert!(epoch("2026-11-19T10:00:00") > 0.0);
        assert_eq!(epoch("2026-11-19T10:00:00.000Z"), 1_795_082_400.0);
        assert_eq!(epoch("nope"), 0.0);
    }

    #[test]
    fn update_and_find() {
        let dir = std::env::temp_dir().join(format!("wb-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let reg = Registry::of_root(&dir, Path::new("C:/x/_work"));
        assert!(reg.load().is_empty());
        reg.update(|ss| ss.push(Session { id: "a".into(), items: vec!["wi1".into()], ..Default::default() }))
            .unwrap();
        let s = reg.modify("a", |s| s.state = State::Paused).unwrap().unwrap();
        assert_eq!(s.state, State::Paused);
        assert!(reg.modify("zzz", |_| ()).unwrap().is_none());
        let raw: Json = serde_json::from_str(&std::fs::read_to_string(reg.path()).unwrap()).unwrap();
        assert_eq!(raw["sessions"][0]["items"][0], "wi1");
        assert_eq!(raw["sessions"][0]["state"], "paused");
        assert_eq!(Registry::find(&dir, "a").unwrap().path(), reg.path());
        assert!(Registry::find(&dir, "b").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
