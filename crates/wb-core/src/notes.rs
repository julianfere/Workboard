//! Notas personales: archivos markdown en una carpeta configurable (por defecto `<config>/notas`; puede
//! ser una carpeta de un vault de Obsidian). Nunca se escriben dentro de los proyectos. El frontmatter
//! vincula la nota a items (por id, o por carpeta si el item no tiene id) y marca quién la escribió.
//! El bloque de frontmatter se conserva tal cual al escribir: en un vault de Obsidian puede traer YAML
//! que el parser propio no entiende (listas en bloque, objetos) y no se puede perder.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{CoreError, ErrorKind, Result};
use crate::frontmatter::{self, Meta, Value};
use crate::records::write_atomic;
use crate::text::slugify;
use crate::workflow::NoteRules;
use crate::workspace::{Item, mtime, normalize, rel_posix};
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

    /// Ruta de una nota (siempre un `.md` dentro de la carpeta de notas).
    pub fn resolve(&self, id: &str) -> Result<PathBuf> {
        let folder = self.canonical_folder();
        let p = dunce::canonicalize(folder.join(id)).unwrap_or_else(|_| normalize(&folder.join(id)));
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
        let raw = frontmatter::read(p)?;
        let (meta, body) = frontmatter::split(&raw);
        let block = split_raw(&raw).0.unwrap_or_default();
        let text: Vec<&str> = py::splitlines(&body, false)
            .into_iter()
            .filter(|l| !py::strip(l).is_empty() && !l.starts_with("# "))
            .map(py::strip)
            .collect();
        let items = match meta.get(&self.rules.items_key) {
            Some(Value::List(l)) => l.clone(),
            Some(Value::Text(t)) if !t.is_empty() => vec![t.clone()],
            _ => block_list(&block, &self.rules.items_key),
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
        self.summary(&self.resolve(id)?, true)
    }

    /// La nota existente (o error) y, si se pasó, el mtime con que se abrió: si cambió, falla.
    fn existing(&self, id: &str, expect_mtime: Option<f64>) -> Result<PathBuf> {
        let p = self.resolve(id)?;
        if !p.is_file() {
            return Err(CoreError::new(
                ErrorKind::NotFound,
                "core.note.missing",
                vec![("id", id.to_string())],
            ));
        }
        if let Some(expect) = expect_mtime
            && (std::fs::metadata(&p).map(|m| mtime(&m)).unwrap_or(0.0) - expect).abs() > 1e-3
        {
            return Err(CoreError::new(ErrorKind::Conflict, "core.note.changed", vec![]));
        }
        Ok(p)
    }

    /// Guarda el cuerpo (el frontmatter no se toca). Si cambió en disco desde que se abrió, falla.
    pub fn save(&self, id: &str, body: &str, expect_mtime: Option<f64>) -> Result<Note> {
        let p = self.existing(id, expect_mtime)?;
        let (block, _) = split_raw(&frontmatter::read(&p)?);
        write_atomic(&p, &join_raw(block.as_deref(), body))?;
        self.read(id)
    }

    /// Cambia el título: la primera línea `# …` (si no hay, la agrega arriba). El archivo conserva su
    /// nombre para no romper los links de Obsidian que apuntan a la nota.
    pub fn set_title(&self, id: &str, title: &str, expect_mtime: Option<f64>) -> Result<Note> {
        let title = py::split_ws(title).collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            return Err(CoreError::invalid("core.note.emptyTitle", vec![]));
        }
        let p = self.existing(id, expect_mtime)?;
        let (block, body) = split_raw(&frontmatter::read(&p)?);
        let mut lines: Vec<String> = py::splitlines(&body, false).into_iter().map(String::from).collect();
        match lines.iter().position(|l| l.starts_with("# ")) {
            Some(k) => lines[k] = format!("# {title}"),
            None => {
                lines.splice(0..0, [format!("# {title}"), String::new()]);
            }
        }
        write_atomic(&p, &join_raw(block.as_deref(), &(lines.join("\n") + "\n")))?;
        self.read(id)
    }

    /// Vincula (`add`) o desvincula un item (`key`: su id o su carpeta). Solo se reescribe la clave de
    /// items del frontmatter; el resto del bloque queda igual.
    pub fn link(&self, id: &str, key: &str, add: bool) -> Result<Note> {
        let key = py::strip(key);
        if key.is_empty() {
            return Err(CoreError::invalid("core.note.emptyLink", vec![]));
        }
        let p = self.existing(id, None)?;
        let note = self.summary(&p, false)?;
        let mut items: Vec<String> = note.items.into_iter().filter(|w| w != key).collect();
        if add {
            items.push(key.to_string());
        }
        let (block, body) = split_raw(&frontmatter::read(&p)?);
        let mut block = block.unwrap_or_default();
        set_block_value(&mut block, &self.rules.items_key, &Value::List(items));
        write_atomic(&p, &join_raw(Some(&block), &body))?;
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

/// Frontmatter crudo (las líneas entre los `---`, con su fin de línea) y el cuerpo, con el mismo corte
/// que `frontmatter::split`. Sin bloque, `None`.
fn split_raw(text: &str) -> (Option<Vec<String>>, String) {
    let lines = py::splitlines(text, true);
    if lines.first().is_none_or(|l| py::strip(l) != "---") {
        return (None, text.to_string());
    }
    match (1..lines.len()).find(|&i| py::strip(lines[i]) == "---") {
        Some(i) => (
            Some(lines[1..i].iter().map(|l| l.to_string()).collect()),
            lines[i + 1..].concat().trim_start_matches('\n').to_string(),
        ),
        None => (None, text.to_string()),
    }
}

/// Vuelve a armar el archivo con el bloque tal cual. Un bloque vacío no se escribe.
fn join_raw(block: Option<&[String]>, body: &str) -> String {
    let Some(block) = block.filter(|b| b.iter().any(|l| !py::strip(l).is_empty())) else {
        return body.to_string();
    };
    let mut out = String::from("---\n");
    for l in block {
        out.push_str(l);
        if !l.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

/// Línea `clave:` de primer nivel del bloque y cuántas líneas ocupa (la de la clave más las de una
/// lista en bloque o un valor anidado: indentadas o que empiezan con `-`).
fn find_key(block: &[String], key: &str) -> Option<(usize, usize)> {
    let at = block.iter().position(|l| {
        !l.starts_with([' ', '\t']) && l.split_once(':').is_some_and(|(k, _)| py::strip(k) == key)
    })?;
    let more = block[at + 1..].iter().take_while(|l| l.starts_with([' ', '\t', '-'])).count();
    Some((at, 1 + more))
}

/// Valores de una lista en bloque de YAML (`clave:` y debajo `  - valor`), como escribe Obsidian.
fn block_list(block: &[String], key: &str) -> Vec<String> {
    let Some((at, len)) = find_key(block, key) else { return vec![] };
    block[at + 1..at + len]
        .iter()
        .filter_map(|l| py::strip(l).strip_prefix('-'))
        .map(|v| py::strip(v).trim_matches(['"', '\'']).to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

/// Reemplaza (o agrega al final) una clave del bloque sin tocar las demás. Un valor vacío la saca.
fn set_block_value(block: &mut Vec<String>, key: &str, value: &Value) {
    let line = (!value.is_empty()).then(|| format!("{key}: {}\n", frontmatter::format_value(value)));
    match find_key(block, key) {
        Some((at, len)) => {
            block.splice(at..at + len, line);
        }
        None => {
            if let Some(last) = block.last_mut()
                && !last.ends_with('\n')
            {
                last.push('\n');
            }
            block.extend(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> Notes {
        let d = std::env::temp_dir().join(format!("wb-notes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Notes::new(d, &Workflow::preset("classic-es").unwrap())
    }

    const OBSIDIAN: &str = "---\ntags:\n  - trabajo\n  - sql\naliases: [Redondeo]\ncssclasses:\n  - wide\nwis:\n  - \"589488\"\n  - wi123-otra\n---\n\nTexto con [[otra nota]].\n";

    #[test]
    fn obsidian_vault_keeps_foreign_frontmatter_and_skips_hidden_folders() {
        let n = store("vault");
        std::fs::create_dir_all(n.folder().join(".obsidian")).unwrap();
        std::fs::write(n.folder().join(".obsidian/workspace.md"), "# no es nota").unwrap();
        std::fs::create_dir_all(n.folder().join("Proyectos/2026")).unwrap();
        std::fs::write(n.folder().join("Proyectos/2026/Redondeo.md"), OBSIDIAN).unwrap();
        std::fs::write(n.folder().join("suelta.md"), "sin frontmatter ni título\n").unwrap();

        let list = n.list();
        let ids: Vec<&str> = list.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(list.len(), 2, "{ids:?}");
        let note = n.read("Proyectos/2026/Redondeo.md").unwrap();
        assert_eq!(note.items, ["589488", "wi123-otra"]);
        assert_eq!(note.title, "Redondeo");
        assert_eq!(n.read("suelta.md").unwrap().title, "suelta");

        // Guardar el cuerpo no toca el bloque ajeno.
        n.save(&note.id, "Otro texto.\n", Some(note.mtime)).unwrap();
        let text = std::fs::read_to_string(n.folder().join("Proyectos/2026/Redondeo.md")).unwrap();
        assert!(
            text.starts_with(
                "---\ntags:\n  - trabajo\n  - sql\naliases: [Redondeo]\ncssclasses:\n  - wide\n"
            ),
            "{text}"
        );
        assert!(text.ends_with("---\nOtro texto.\n"), "{text}");

        // Vincular reescribe solo la clave de items (la lista en bloque pasa a lista en línea).
        let linked = n.link(&note.id, "600001", true).unwrap();
        assert_eq!(linked.items, ["589488", "wi123-otra", "600001"]);
        let unlinked = n.link(&note.id, "589488", false).unwrap();
        assert_eq!(unlinked.items, ["wi123-otra", "600001"]);
        let text = std::fs::read_to_string(n.folder().join("Proyectos/2026/Redondeo.md")).unwrap();
        assert_eq!(
            text,
            "---\ntags:\n  - trabajo\n  - sql\naliases: [Redondeo]\ncssclasses:\n  - wide\nwis: [wi123-otra, 600001]\n---\nOtro texto.\n"
        );
        // Sin items, la clave sale; el resto del bloque queda.
        n.link(&note.id, "wi123-otra", false).unwrap();
        let last = n.link(&note.id, "600001", false).unwrap();
        assert!(last.items.is_empty());
        let text = std::fs::read_to_string(n.folder().join("Proyectos/2026/Redondeo.md")).unwrap();
        assert!(!text.contains("wis:") && text.contains("cssclasses:\n  - wide\n---\n"), "{text}");
    }

    #[test]
    fn link_creates_the_block_when_missing() {
        let n = store("link");
        std::fs::write(n.folder().join("a.md"), "# A\n\ncuerpo\n").unwrap();
        let note = n.link("a.md", "589488", true).unwrap();
        assert_eq!(note.items, ["589488"]);
        assert_eq!(
            std::fs::read_to_string(n.folder().join("a.md")).unwrap(),
            "---\nwis: [589488]\n---\n# A\n\ncuerpo\n"
        );
        assert_eq!(n.link("a.md", " ", true).unwrap_err().code, "core.note.emptyLink");
        assert_eq!(n.link("nada.md", "1", true).unwrap_err().code, "core.note.missing");
    }

    #[test]
    fn set_title_keeps_the_file_name() {
        let n = store("title");
        let note = n.create("Primera idea", &["589488".into()], "claude", "texto", "2026-11-20").unwrap();
        assert_eq!(note.id, "primera-idea.md");
        assert_eq!(note.author, "claude");
        let renamed = n.set_title(&note.id, "  Otra   cosa ", Some(note.mtime)).unwrap();
        assert_eq!((renamed.id.as_str(), renamed.title.as_str()), ("primera-idea.md", "Otra cosa"));
        assert_eq!(renamed.items, ["589488"]);
        let text = std::fs::read_to_string(n.folder().join("primera-idea.md")).unwrap();
        assert!(text.contains("autor: claude") && text.contains("# Otra cosa\n\ntexto\n"), "{text}");

        // Sin línea `# …`, la agrega arriba.
        std::fs::write(n.folder().join("b.md"), "---\nx: 1\n---\nsolo texto\n").unwrap();
        n.set_title("b.md", "B", None).unwrap();
        assert_eq!(
            std::fs::read_to_string(n.folder().join("b.md")).unwrap(),
            "---\nx: 1\n---\n# B\n\nsolo texto\n"
        );

        assert_eq!(n.set_title("b.md", "  ", None).unwrap_err().code, "core.note.emptyTitle");
        assert_eq!(n.set_title("b.md", "C", Some(1.0)).unwrap_err().code, "core.note.changed");
        assert_eq!(n.set_title("../fuera.md", "C", None).unwrap_err().code, "core.note.outside");
    }
}
