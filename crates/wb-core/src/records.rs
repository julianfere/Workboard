//! Datos que viven en la carpeta de config, fuera de los proyectos: qué scripts se corrieron en qué
//! ambiente (`runs.json`) y la bandeja de tareas sin item. Mismo formato que la v1.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::tasks::{Task, TaskSection};
use crate::{Workflow, frontmatter, py};

/// `{ambiente: timestamp}` de un archivo.
pub type Marks = BTreeMap<String, f64>;
type Runs = BTreeMap<String, BTreeMap<String, Marks>>;

/// Escritura atómica (tmp + rename).
pub(crate) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text.as_bytes())?;
    std::fs::rename(&tmp, path)
}

/// JSON con sangría de un espacio, como `json.dumps(indent=1)` de la v1.
fn to_json(value: &impl Serialize) -> String {
    let mut out = Vec::new();
    let mut ser =
        serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b" "));
    value.serialize(&mut ser).expect("serializable");
    String::from_utf8(out).expect("utf-8")
}

/// Registro de scripts corridos: `{raíz: {ruta relativa: {ambiente: timestamp}}}`. Lo escriben la app y
/// el MCP; las escrituras son atómicas y cortas.
pub struct RunLog {
    path: PathBuf,
}

impl RunLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Clave de una raíz: su ruta canónica (en Windows, en minúsculas como `os.path.normcase`).
    pub fn root_key(root: &Path) -> String {
        let p = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let s = p.display().to_string();
        if cfg!(windows) { s.replace('/', "\\").to_lowercase() } else { s }
    }

    fn load(&self) -> Runs {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn of_root(&self, root: &Path) -> BTreeMap<String, Marks> {
        self.load().remove(&Self::root_key(root)).unwrap_or_default()
    }

    /// Marca (o desmarca) `rel` como corrido en `env`. Devuelve las marcas que le quedan.
    pub fn mark(
        &self,
        wf: &Workflow,
        root: &Path,
        rel: &str,
        env: &str,
        on: bool,
        ts: Option<f64>,
    ) -> Result<Marks> {
        if !wf.environments.iter().any(|e| e.id == env) {
            let valid: Vec<&str> = wf.environments.iter().map(|e| e.id.as_str()).collect();
            return Err(CoreError::invalid(
                "core.run.badEnv",
                vec![("env", py::repr(env)), ("valid", valid.join(", "))],
            ));
        }
        let mut data = self.load();
        let runs = data.entry(Self::root_key(root)).or_default();
        let entry = runs.entry(rel.to_string()).or_default();
        if on {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64());
            entry.insert(env.to_string(), ts.unwrap_or(now));
        } else {
            entry.remove(env);
        }
        let result = entry.clone();
        if result.is_empty() {
            runs.remove(rel);
        }
        write_atomic(&self.path, &to_json(&data))?;
        Ok(result)
    }

    /// Las marcas siguen a la carpeta cuando se mueve (agrupar, eliminar, deshacer).
    pub fn move_prefix(&self, root: &Path, old: &str, new: &str) -> Result<()> {
        let mut data = self.load();
        let key = Self::root_key(root);
        let Some(runs) = data.get(&key) else { return Ok(()) };
        let (old, new) =
            (format!("{}/", old.trim_end_matches('/')), format!("{}/", new.trim_end_matches('/')));
        let moved: BTreeMap<String, Marks> = runs
            .iter()
            .map(|(k, v)| {
                (k.strip_prefix(&old).map_or_else(|| k.clone(), |rest| format!("{new}{rest}")), v.clone())
            })
            .collect();
        if &moved != runs {
            data.insert(key, moved);
            write_atomic(&self.path, &to_json(&data))?;
        }
        Ok(())
    }
}

/// Bandeja de tareas sin item: un markdown con la sección de tareas, en la carpeta de config.
pub struct Inbox {
    path: PathBuf,
    section: TaskSection,
    default_text: String,
}

impl Inbox {
    pub fn new(path: PathBuf, wf: &Workflow) -> Self {
        Self {
            path,
            section: TaskSection::new(&wf.tasks.heading, wf.tasks.max_len),
            default_text: format!("# {}\n", wf.inbox_title),
        }
    }

    fn text(&self) -> Result<String> {
        if self.path.exists() { Ok(frontmatter::read(&self.path)?) } else { Ok(self.default_text.clone()) }
    }

    fn write(&self, f: impl FnOnce(&TaskSection, &str) -> Result<String>) -> Result<Vec<Task>> {
        let new = f(&self.section, &self.text()?)?;
        write_atomic(&self.path, &new)?;
        self.tasks()
    }

    pub fn tasks(&self) -> Result<Vec<Task>> {
        Ok(self.section.parse(&self.text()?))
    }

    pub fn add(&self, text: &str) -> Result<Vec<Task>> {
        let text = self.section.clean(text)?;
        self.write(|s, body| Ok(s.add(body, &text)))
    }

    pub fn set(&self, index: i64, done: bool, expect: Option<&str>) -> Result<Vec<Task>> {
        self.write(|s, body| s.set(body, index, done, expect))
    }

    pub fn remove(&self, index: i64, expect: Option<&str>) -> Result<Vec<Task>> {
        self.write(|s, body| s.remove(body, index, expect))
    }

    pub fn edit(&self, index: i64, text: &str, expect: Option<&str>) -> Result<Vec<Task>> {
        let text = self.section.clean(text)?;
        self.write(|s, body| s.edit(body, index, &text, expect))
    }

    pub fn clear_done(&self) -> Result<Vec<Task>> {
        self.write(|s, body| Ok(s.clear_done(body).0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-records-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn runs_mark_and_move() {
        let dir = temp("runs");
        let wf = Workflow::preset("classic-es").unwrap();
        let log = RunLog::new(dir.join("runs.json"));
        assert_eq!(log.mark(&wf, &dir, "a/x.sql", "qa", true, Some(1.0)).unwrap().len(), 1);
        log.mark(&wf, &dir, "a/x.sql", "dev", true, Some(2.0)).unwrap();
        assert!(log.mark(&wf, &dir, "a/x.sql", "uat", true, None).is_err());
        log.move_prefix(&dir, "a", "b").unwrap();
        let runs = log.of_root(&dir);
        assert_eq!(runs["b/x.sql"].keys().collect::<Vec<_>>(), ["dev", "qa"]);
        log.mark(&wf, &dir, "b/x.sql", "qa", false, None).unwrap();
        log.mark(&wf, &dir, "b/x.sql", "dev", false, None).unwrap();
        assert!(log.of_root(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn inbox() {
        let dir = temp("inbox");
        let wf = Workflow::preset("classic-es").unwrap();
        let inbox = Inbox::new(dir.join("inbox.md"), &wf);
        inbox.add("  primera  ").unwrap();
        inbox.add("segunda").unwrap();
        inbox.set(0, true, Some("primera")).unwrap();
        assert_eq!(inbox.clear_done().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(dir.join("inbox.md")).unwrap(),
            "# Bandeja\n\n## Pendientes\n\n- [ ] segunda\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
