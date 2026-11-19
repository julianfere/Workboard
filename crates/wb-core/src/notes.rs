//! Notas personales: archivos markdown en una carpeta configurable (por defecto `<config>/notas`; puede
//! ser una carpeta de un vault de Obsidian). Nunca se escriben dentro de los proyectos. El frontmatter
//! vincula la nota a items (por id, o por carpeta si el item no tiene id) y marca quién la escribió.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{CoreError, ErrorKind, Result};
use crate::frontmatter::{self, Meta, Value};
use crate::records::write_atomic;
use crate::text::slugify;
use crate::workflow::NoteRules;
use crate::workspace::{Item, mtime, rel_posix};
use crate::{Workflow, py};

pub const MAX_NOTES: usize = 2000;
pub const EXCERPT_LEN: usize = 220;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// Ruta relativa a la carpeta de notas (`hallazgo.md`).
    pub id: String,
    pub title: String,
    /// Items vinculados.
    pub items: Vec<String>,
    pub author: String,
    pub mtime: f64,
    pub excerpt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// Cómo se nombra un item dentro de las notas: su id, o la carpeta si no tiene.
pub fn item_key(item: &Item) -> String {
    item.ids.first().cloned().unwrap_or_else(|| item.name.clone())
}

pub struct Notes {
    folder: PathBuf,
    rules: NoteRules,
}

fn title_of(body: &str, path: &Path) -> String {
    py::splitlines(body, false)
        .iter()
        .find(|l| l.starts_with("# "))
        .map(|l| py::strip(&l[2..]).to_string())
        .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}

impl Notes {
    pub fn new(folder: PathBuf, wf: &Workflow) -> Self {
        Self { folder, rules: wf.notes.clone() }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn canonical_folder(&self) -> PathBuf {
        dunce::canonicalize(&self.folder).unwrap_or_else(|_| self.folder.clone())
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        let folder = self.canonical_folder();
        let p = dunce::canonicalize(folder.join(id)).unwrap_or_else(|_| folder.join(id));
        let is_md = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"));
        if !is_md || !p.starts_with(&folder) || p == folder {
            return Err(CoreError::new(
                ErrorKind::Permission,
                "core.note.outside",
                vec![("id", id.to_string())],
            ));
        }
        Ok(p)
    }

    fn summary(&self, p: &Path, with_body: bool) -> Result<Note> {
        let (meta, body) = frontmatter::split(&frontmatter::read(p)?);
        let text: Vec<&str> = py::splitlines(&body, false)
            .into_iter()
            .filter(|l| !py::strip(l).is_empty() && !l.starts_with("# "))
            .map(py::strip)
            .collect();
        let items = match meta.get(&self.rules.items_key) {
            Some(Value::List(l)) => l.clone(),
            Some(Value::Text(t)) if !t.is_empty() => vec![t.clone()],
            _ => vec![],
        };
        Ok(Note {
            id: rel_posix(&self.canonical_folder(), p),
            title: title_of(&body, p),
            items,
            author: meta.get(&self.rules.author_key).and_then(Value::as_text).unwrap_or("").to_string(),
            mtime: std::fs::metadata(p).map(|m| mtime(&m)).unwrap_or(0.0),
            excerpt: text.join(" ").chars().take(EXCERPT_LEN).collect(),
            body: with_body.then_some(body),
        })
    }

    /// Todas las notas (sin carpetas ocultas como `.obsidian`), la más reciente primero.
    pub fn list(&self) -> Vec<Note> {
        let mut out = Vec::new();
        let mut stack = vec![self.canonical_folder()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if p.is_dir() {
                    if !name.starts_with('.') {
                        stack.push(p);
                    }
                } else if name.to_lowercase().ends_with(".md")
                    && out.len() < MAX_NOTES
                    && let Ok(n) = self.summary(&p, false)
                {
                    out.push(n);
                }
            }
        }
        out.sort_by(|a, b| b.mtime.total_cmp(&a.mtime));
        out
    }

    pub fn read(&self, id: &str) -> Result<Note> {
        self.summary(&self.path(id)?, true)
    }

    /// Guarda el cuerpo (el frontmatter no se toca). Si cambió en disco desde que se abrió, falla.
    pub fn save(&self, id: &str, body: &str, expect_mtime: Option<f64>) -> Result<Note> {
        let p = self.path(id)?;
        if let Some(expect) = expect_mtime
            && (std::fs::metadata(&p).map(|m| mtime(&m)).unwrap_or(0.0) - expect).abs() > 1e-3
        {
            return Err(CoreError::new(ErrorKind::Conflict, "core.note.changed", vec![]));
        }
        let (meta, _) = frontmatter::split(&frontmatter::read(&p)?);
        write_atomic(&p, &frontmatter::join(&meta, body))?;
        self.read(id)
    }

    /// Nota nueva (nombre de archivo a partir del título, sin pisar otra). `today` = `AAAA-MM-DD`.
    pub fn create(
        &self,
        title: &str,
        items: &[String],
        author: &str,
        body: &str,
        today: &str,
    ) -> Result<Note> {
        let title = py::split_ws(title).collect::<Vec<_>>().join(" ");
        let title = if title.is_empty() { self.rules.untitled.clone() } else { title };
        std::fs::create_dir_all(&self.folder)?;
        let base = slugify(&title, 60, "wi");
        let mut p = self.folder.join(format!("{base}.md"));
        let mut n = 2;
        while p.exists() {
            p = self.folder.join(format!("{base}-{n}.md"));
            n += 1;
        }
        let mut meta = Meta::new();
        if !items.is_empty() {
            meta.insert(self.rules.items_key.clone(), Value::List(items.to_vec()));
        }
        if !author.is_empty() {
            meta.insert(self.rules.author_key.clone(), Value::Text(author.into()));
        }
        meta.insert(self.rules.created_key.clone(), Value::Text(today.into()));
        let body = py::strip(body);
        let text = if body.is_empty() { format!("# {title}\n\n") } else { format!("# {title}\n\n{body}\n") };
        write_atomic(&p, &frontmatter::join(&meta, &text))?;
        self.read(&p.file_name().unwrap().to_string_lossy())
    }
}
