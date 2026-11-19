//! Escrituras dentro de la carpeta de trabajos. La única escritura de README es `write_readme` (la usan la
//! UI y el MCP): relee si el archivo cambió mientras se calculaba, deja una copia antes de pisarlo y
//! escribe de forma atómica. Mover, eliminar y renombrar son siempre un rename dentro de la raíz: nunca
//! se borra ni se copia contenido.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde::Serialize;

use crate::error::{CoreError, ErrorKind, Result};
use crate::frontmatter::{self, Meta, Value};
use crate::workspace::{Item, README, mtime};
use crate::{Workspace, py};

pub const BACKUP_KEEP: usize = 20;
const WRITE_RETRIES: usize = 3;

static FOLDER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,120}$").unwrap());
static KEY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z_]+$").unwrap());
static BACKUP_ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{8}-\d{6}-\d{9}$").unwrap());

/// Lo que devuelve un cambio de README: frontmatter y cuerpo nuevos, o el texto completo (restaurar).
pub enum Change {
    Parts(Meta, String),
    Text(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Version {
    pub id: String,
    pub ts: f64,
    pub size: u64,
}

fn err(kind: ErrorKind, code: &'static str, params: Vec<(&'static str, String)>) -> CoreError {
    CoreError::new(kind, code, params)
}

/// (mtime en ns, tamaño) del archivo, o None si no existe: si cambia, otro proceso escribió.
fn stamp(path: &Path) -> Option<(u128, u64)> {
    let m = std::fs::metadata(path).ok()?;
    let ns = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    Some((ns, m.len()))
}

/// `os.path.normcase`: en Windows, minúsculas y `\`.
fn normcase(p: &Path) -> String {
    let s = p.display().to_string();
    if cfg!(windows) { s.replace('/', "\\").to_lowercase() } else { s }
}

impl Workspace {
    /// Única escritura de README. `change(meta, cuerpo)` arma la versión nueva; si el archivo cambió en
    /// disco mientras tanto, se relee y se vuelve a aplicar (hasta 3 veces).
    pub fn write_readme(
        &self,
        name: &str,
        mut change: impl FnMut(Meta, String) -> Result<Change>,
    ) -> Result<Item> {
        let folder = self.folder_of(name)?;
        let readme = folder.join(README);
        for _ in 0..WRITE_RETRIES {
            let before = stamp(&readme);
            let old = if before.is_some() { Some(frontmatter::read(&readme)?) } else { None };
            let (meta, body) = match &old {
                Some(text) => frontmatter::split(text),
                None => (Meta::new(), format!("# {name}\n")),
            };
            let new = match change(meta, body)? {
                Change::Parts(meta, body) => frontmatter::join(&meta, &body),
                Change::Text(text) => text,
            };
            if old.as_deref() == Some(new.as_str()) {
                return self.build_item(&folder);
            }
            if stamp(&readme) != before {
                continue; // otro proceso escribió mientras calculábamos: releer
            }
            if let Some(old) = &old {
                self.backup(&folder, old)?;
            }
            let tmp = folder.join("README.md.tmp");
            std::fs::write(&tmp, new.as_bytes())?;
            std::fs::rename(&tmp, &readme)?; // atómico: nunca queda un README a medias
            let old_meta = old.as_deref().map(|t| frontmatter::split(t).0).unwrap_or_default();
            self.log_state(&folder, &old_meta, &new);
            return self.build_item(&folder);
        }
        Err(err(ErrorKind::Conflict, "core.write.conflict", vec![]))
    }

    /// Cada cambio de estado va a `<config>/state_log.jsonl` (formato de la v1). Nunca frena la escritura.
    fn log_state(&self, folder: &Path, old_meta: &Meta, new_text: &str) {
        let Some(backups) = self.backup_dir() else { return };
        let key = &self.workflow().fields.status;
        let status = |m: &Meta| {
            m.get(key).and_then(Value::as_text).map(|s| py::strip(s).to_string()).unwrap_or_default()
        };
        let (before, after) = (status(old_meta), status(&frontmatter::split(new_text).0));
        if before == after {
            return;
        }
        let ts = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
            * 10.0)
            .round()
            / 10.0;
        let row = serde_json::json!({
            "ts": ts,
            "root": normcase(self.root()),
            "wi": self.rel(folder),
            "de": before,
            "a": after,
            "session": std::env::var("WORKBOARD_SESSION").ok().filter(|s| !s.is_empty()),
        });
        let path = backups.parent().unwrap_or(backups).join("state_log.jsonl");
        let _ = (|| -> std::io::Result<()> {
            use std::io::Write;
            std::fs::create_dir_all(path.parent().unwrap())?;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
            writeln!(f, "{row}")
        })();
    }

    /// Cambia campos del frontmatter ("" o [] = borrar). Pasar a un estado cerrado tilda las tareas abiertas.
    pub fn update_meta(&self, name: &str, fields: &IndexMap<String, Value>) -> Result<Item> {
        if let Some(key) = fields.keys().find(|k| !KEY_RE.is_match(k)) {
            return Err(CoreError::invalid("core.write.badKey", vec![("key", key.clone())]));
        }
        let status_key = self.workflow().fields.status.clone();
        self.write_readme(name, |mut meta, mut body| {
            let closed =
                |v: Option<&Value>| v.and_then(Value::as_text).is_some_and(|s| self.workflow().is_closed(s));
            if closed(fields.get(&status_key)) && !closed(meta.get(&status_key)) {
                body = self.task_section().close_all(&body);
            }
            for (key, value) in fields {
                if value.is_empty() {
                    meta.shift_remove(key);
                } else {
                    meta.insert(key.clone(), value.clone());
                }
            }
            Ok(Change::Parts(meta, body))
        })
    }

    pub fn add_task(&self, name: &str, text: &str) -> Result<Item> {
        let text = self.task_section().clean(text)?;
        self.write_readme(name, |meta, body| Ok(Change::Parts(meta, self.task_section().add(&body, &text))))
    }

    /// Tilda o destilda la tarea `index`. `expect` = texto que vio quien llama: si no coincide, falla.
    pub fn set_task(&self, name: &str, index: i64, done: bool, expect: Option<&str>) -> Result<Item> {
        self.write_readme(name, |meta, body| {
            Ok(Change::Parts(meta, self.task_section().set(&body, index, done, expect)?))
        })
    }

    pub fn remove_task(&self, name: &str, index: i64, expect: Option<&str>) -> Result<Item> {
        self.write_readme(name, |meta, body| {
            Ok(Change::Parts(meta, self.task_section().remove(&body, index, expect)?))
        })
    }

    pub fn edit_task(&self, name: &str, index: i64, text: &str, expect: Option<&str>) -> Result<Item> {
        let text = self.task_section().clean(text)?;
        self.write_readme(name, |meta, body| {
            Ok(Change::Parts(meta, self.task_section().edit(&body, index, &text, expect)?))
        })
    }

    // ---------- mover carpetas ----------

    fn group_prefix(&self) -> &str {
        &self.workflow().folders.group_prefix
    }

    fn is_root_group(&self, p: &Path) -> bool {
        p.parent() == Some(self.root()) && self.is_group(p)
    }

    fn rename_dir(&self, src: &Path, target: &Path, old_name: &str) -> Result<String> {
        std::fs::rename(src, target)?;
        let new = self.rel(target);
        self.move_backups(old_name, &new);
        Ok(new)
    }

    /// Mueve el item `name` a la raíz (`dest` = "") o dentro del grupo `dest`. Devuelve el nombre nuevo.
    pub fn move_folder(&self, name: &str, dest: &str) -> Result<String> {
        let src = self.folder_of(name)?;
        if self.is_root_group(&src) {
            if dest.is_empty() {
                return Ok(name.to_string());
            }
            return Err(CoreError::invalid("core.move.groupInGroup", vec![]));
        }
        let parent = if dest.is_empty() {
            self.root().to_path_buf()
        } else {
            let p = self.folder_of(dest)?;
            if !self.is_root_group(&p) {
                return Err(CoreError::invalid("core.move.notGroup", vec![("dest", dest.to_string())]));
            }
            p
        };
        if src.parent() == Some(parent.as_path()) {
            return Ok(name.to_string());
        }
        let target = parent.join(src.file_name().unwrap());
        if target.exists() {
            return Err(err(ErrorKind::Exists, "core.move.exists", vec![("path", self.rel(&target))]));
        }
        self.rename_dir(&src, &target, name)
    }

    /// Eliminar = mover a la carpeta de eliminados (se vuelve con `untrash`). Devuelve la ruta nueva.
    pub fn trash(&self, name: &str) -> Result<String> {
        let src = self.folder_of(name)?;
        let bin = self.root().join(&self.workflow().folders.deleted_dir);
        std::fs::create_dir_all(&bin)?;
        let base = src.file_name().unwrap().to_string_lossy().into_owned();
        let mut target = bin.join(&base);
        if target.exists() {
            target = bin.join(format!("{base}-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        }
        self.rename_dir(&src, &target, name)
    }

    /// Deshace `trash`: vuelve `deleted` a `name` (en la raíz o en su grupo, si sigue existiendo).
    pub fn untrash(&self, deleted: &str, name: &str) -> Result<String> {
        let deleted_dir = &self.workflow().folders.deleted_dir;
        let src = self.safe_path(deleted)?;
        if src.parent() != Some(self.root().join(deleted_dir).as_path()) || !src.is_dir() {
            return Err(err(
                ErrorKind::NotFound,
                "core.trash.notDeleted",
                vec![("dir", deleted_dir.clone()), ("path", deleted.to_string())],
            ));
        }
        let target = self.safe_path(name)?;
        let parent = target.parent().unwrap_or(self.root());
        if !(parent == self.root() || (parent.parent() == Some(self.root()) && self.is_group(parent))) {
            return Err(CoreError::invalid("core.trash.badTarget", vec![("name", name.to_string())]));
        }
        if !parent.is_dir() {
            return Err(err(ErrorKind::NotFound, "core.trash.groupGone", vec![("name", name.to_string())]));
        }
        if target.exists() {
            return Err(err(ErrorKind::Exists, "core.trash.exists", vec![("name", name.to_string())]));
        }
        std::fs::rename(&src, &target)?;
        self.move_backups(deleted, name);
        Ok(name.to_string())
    }

    fn check_folder_name(&self, base: &str) -> Result<()> {
        if !FOLDER_RE.is_match(base) || self.workflow().is_ignored_folder(base) {
            return Err(CoreError::invalid("core.folder.badName", vec![("name", py::repr(base))]));
        }
        Ok(())
    }

    /// Renombra un item o grupo en su lugar. Un grupo tiene que seguir siendo grupo (y un item, item).
    pub fn rename_folder(&self, name: &str, new_base: &str) -> Result<String> {
        let src = self.folder_of(name)?;
        let new_base = py::strip(new_base);
        self.check_folder_name(new_base)?;
        let was_group = self.is_root_group(&src);
        let prefix = self.group_prefix().to_lowercase();
        if was_group != new_base.to_lowercase().starts_with(&prefix) {
            let code = if was_group { "core.folder.groupMustStay" } else { "core.folder.itemNotGroup" };
            return Err(CoreError::invalid(code, vec![("prefix", self.group_prefix().to_string())]));
        }
        if src.file_name().is_some_and(|n| n == new_base) {
            return Ok(name.to_string());
        }
        let target = src.parent().unwrap().join(new_base);
        if target.exists() && normcase(&target) != normcase(&src) {
            return Err(err(ErrorKind::Exists, "core.move.exists", vec![("path", self.rel(&target))]));
        }
        self.rename_dir(&src, &target, name)
    }

    /// Carpeta nueva de item en la raíz (nombre simple que no exista). Devuelve su ruta.
    pub fn create_folder(&self, name: &str) -> Result<PathBuf> {
        if !FOLDER_RE.is_match(name) {
            return Err(CoreError::invalid("core.folder.badName", vec![("name", py::repr(name))]));
        }
        let p = self.root().join(name);
        if p.exists() {
            return Err(err(ErrorKind::Exists, "core.folder.exists", vec![("name", name.to_string())]));
        }
        std::fs::create_dir(&p)?;
        Ok(p)
    }

    // ---------- versiones anteriores (backups) ----------

    /// `<backups>/<raíz>-<sha1[:12]>/<item>`: la misma ubicación que la v1.
    fn backup_folder(&self, name: &str) -> Option<PathBuf> {
        let dir = self.backup_dir()?;
        let hash = sha1_smol::Sha1::from(normcase(self.root()).as_bytes()).digest().to_string();
        let root_name = self.root().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Some(dir.join(format!("{root_name}-{}", &hash[..12])).join(name))
    }

    fn backup(&self, folder: &Path, text: &str) -> Result<()> {
        let Some(dir) = self.backup_folder(&self.rel(folder)) else { return Ok(()) };
        std::fs::create_dir_all(&dir)?;
        // Nombre por fecha local + nanosegundos; si se repite (resolución del reloj), se avanza.
        let now = chrono::Local::now();
        let mut ns = u64::from(now.timestamp_subsec_nanos());
        let stamp = now.format("%Y%m%d-%H%M%S").to_string();
        let target = loop {
            let t = dir.join(format!("{stamp}-{ns:09}.md"));
            if !t.exists() {
                break t;
            }
            ns += 1;
        };
        std::fs::write(&target, text.as_bytes())?;
        let mut copies = self.version_files(&dir);
        copies.sort();
        let excess = copies.len().saturating_sub(BACKUP_KEEP);
        for old in &copies[..excess] {
            let _ = std::fs::remove_file(old);
        }
        Ok(())
    }

    fn version_files(&self, dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "md")).collect()
            })
            .unwrap_or_default()
    }

    /// Los backups siguen a la carpeta cuando se mueve. Es un extra: si falla, no frena el movimiento.
    fn move_backups(&self, old: &str, new: &str) {
        if let (Some(a), Some(b)) = (self.backup_folder(old), self.backup_folder(new))
            && a.is_dir()
            && !b.exists()
        {
            let _ = std::fs::create_dir_all(b.parent().unwrap()).and_then(|()| std::fs::rename(&a, &b));
        }
    }

    pub fn list_versions(&self, name: &str) -> Result<Vec<Version>> {
        self.folder_of(name)?;
        let Some(dir) = self.backup_folder(name) else { return Ok(vec![]) };
        let mut out: Vec<Version> = self
            .version_files(&dir)
            .into_iter()
            .filter_map(|p| {
                let id = p.file_stem()?.to_string_lossy().into_owned();
                BACKUP_ID_RE.is_match(&id).then(|| {
                    let meta = std::fs::metadata(&p).ok();
                    let ts = chrono::NaiveDateTime::parse_from_str(&id[..15], "%Y%m%d-%H%M%S")
                        .ok()
                        .and_then(|dt| dt.and_local_timezone(chrono::Local).earliest())
                        .map_or_else(|| meta.as_ref().map_or(0.0, mtime), |dt| dt.timestamp() as f64);
                    Version { id, ts, size: meta.map_or(0, |m| m.len()) }
                })
            })
            .collect();
        out.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(out)
    }

    fn version_path(&self, name: &str, id: &str) -> Result<PathBuf> {
        self.folder_of(name)?;
        let dir = self.backup_folder(name).filter(|_| BACKUP_ID_RE.is_match(id));
        let Some(dir) = dir else { return Err(CoreError::invalid("core.version.invalid", vec![])) };
        let p = dir.join(format!("{id}.md"));
        if !p.is_file() {
            return Err(err(ErrorKind::NotFound, "core.version.missing", vec![]));
        }
        Ok(p)
    }

    pub fn read_version(&self, name: &str, id: &str) -> Result<String> {
        Ok(frontmatter::read(&self.version_path(name, id)?)?)
    }

    /// Vuelve el README a esa versión. El contenido actual queda a su vez como copia.
    pub fn restore_version(&self, name: &str, id: &str) -> Result<Item> {
        let text = self.read_version(name, id)?;
        self.write_readme(name, |_, _| Ok(Change::Text(text.clone())))
    }
}
