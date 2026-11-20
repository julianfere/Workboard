//! La carpeta de trabajos: un item por subcarpeta (o por subcarpeta de un grupo), con su estado en el
//! frontmatter del README. Se lee entera en cada llamada (sin caché) y nunca se sale de la raíz.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use base64::Engine;
use serde::Serialize;

use crate::error::{CoreError, ErrorKind, Result};
use crate::frontmatter::{self, Meta, Value};
use crate::tasks::{Task, TaskSection};
use crate::text::{self, PlanGap};
use crate::{StateCategory, Workflow, py};

pub const README: &str = "README.md";
pub const MAX_TEXT_BYTES: u64 = 2_000_000;
pub const MAX_MEDIA_BYTES: u64 = 30_000_000;

const TEXT_EXT: &[&str] = &[
    ".md", ".sql", ".txt", ".ps1", ".py", ".json", ".csv", ".cs", ".ts", ".js", ".html", ".xml", ".yml",
    ".yaml", ".log", ".rpt", ".sh", ".bat", ".ini", ".config", ".drawio", ".dio", ".mmd", ".mermaid",
];
const MEDIA_TYPES: &[(&str, &str)] = &[
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".gif", "image/gif"),
    (".webp", "image/webp"),
    (".bmp", "image/bmp"),
    (".svg", "image/svg+xml"),
    (".pdf", "application/pdf"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Item,
    Group,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    /// Ruta relativa a la raíz, con `/`.
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime: f64,
    pub kind: Option<String>,
    pub cat: String,
    /// Se puede mostrar como texto.
    pub text: bool,
    /// Tipo MIME si se puede mostrar como imagen o PDF.
    pub media: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// Clave del item: ruta relativa a la raíz (`wi1-x` o `grp-y/wi1-x`).
    pub name: String,
    pub kind: ItemKind,
    /// Grupo que lo contiene ("" en la raíz).
    pub group: String,
    /// Items de un grupo (`grp-y/wi1-x`).
    pub members: Vec<String>,
    /// Ids de work item (del nombre de la carpeta o del frontmatter).
    pub ids: Vec<String>,
    pub title: String,
    /// Tipo del frontmatter si es uno del flujo, si no "".
    #[serde(rename = "type")]
    pub item_type: String,
    pub meta: Meta,
    pub has_readme: bool,
    /// Sección "cómo deployar" del README.
    pub deploy: String,
    pub tasks: Vec<Task>,
    pub plan_gaps: Vec<PlanGap>,
    pub created: f64,
    /// Última actividad del trabajo (sin contar el README si hay otros archivos).
    pub last: f64,
    pub size: u64,
    pub files: Vec<FileEntry>,
    pub kinds: Vec<String>,
    /// Otras carpetas con alguno de los mismos ids (solo en `list_items`, que compara todas).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dupes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedEntry {
    pub path: String,
    pub base: String,
    pub title: String,
    pub status: String,
    pub deleted: f64,
    pub group: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Media {
    pub mime: &'static str,
    /// Contenido en base64.
    pub data: String,
}

pub struct Workspace {
    root: PathBuf,
    backup_dir: Option<PathBuf>,
    wf: Workflow,
    tasks: TaskSection,
}

fn err(kind: ErrorKind, code: &'static str, params: Vec<(&'static str, String)>) -> CoreError {
    CoreError::new(kind, code, params)
}

pub(crate) fn mtime(meta: &std::fs::Metadata) -> f64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs_f64())
}

fn ctime(meta: &std::fs::Metadata) -> f64 {
    meta.created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or_else(|| mtime(meta), |d| d.as_secs_f64())
}

/// Extensión en minúsculas con el punto (`.sql`), o "" (como `Path.suffix` de Python).
fn suffix(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

pub(crate) fn rel_posix(root: &Path, p: &Path) -> String {
    let rel = p.strip_prefix(root).unwrap_or(p);
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if parts.is_empty() { ".".into() } else { parts.join("/") }
}

/// Normaliza `.` y `..` sin tocar el disco (`Path.resolve` sobre rutas que pueden no existir).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::RootDir | Component::Prefix(_)) | None
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl Workspace {
    /// Abre una raíz existente. `backup_dir`: carpeta de copias de los README (`<config>/backups`).
    pub fn open(root: &Path, wf: Workflow, backup_dir: Option<PathBuf>) -> Result<Self> {
        let canonical = dunce::canonicalize(root).ok().filter(|p| p.is_dir()).ok_or_else(|| {
            err(ErrorKind::NotFound, "core.ws.rootMissing", vec![("path", root.display().to_string())])
        })?;
        let tasks = TaskSection::new(&wf.tasks.heading, wf.tasks.max_len);
        Ok(Self { root: canonical, backup_dir, wf, tasks })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn workflow(&self) -> &Workflow {
        &self.wf
    }

    pub fn task_section(&self) -> &TaskSection {
        &self.tasks
    }

    pub(crate) fn backup_dir(&self) -> Option<&Path> {
        self.backup_dir.as_deref()
    }

    // ---------- carpetas ----------

    fn is_work_folder(&self, p: &Path) -> bool {
        p.is_dir() && p.file_name().is_some_and(|n| !self.wf.is_ignored_folder(&n.to_string_lossy()))
    }

    pub(crate) fn is_group(&self, p: &Path) -> bool {
        self.is_work_folder(p) && p.file_name().is_some_and(|n| self.wf.is_group_folder(&n.to_string_lossy()))
    }

    /// Ruta absoluta de `rel`, que tiene que quedar dentro de la raíz.
    pub fn safe_path(&self, rel: &str) -> Result<PathBuf> {
        let p = normalize(&self.root.join(rel));
        if !p.starts_with(&self.root) {
            return Err(err(ErrorKind::Permission, "core.ws.outsideRoot", vec![("rel", rel.to_string())]));
        }
        Ok(p)
    }

    pub fn rel(&self, p: &Path) -> String {
        rel_posix(&self.root, p)
    }

    /// Carpeta de un item o grupo: hija de la raíz, o hija de un grupo de la raíz.
    pub fn folder_of(&self, name: &str) -> Result<PathBuf> {
        let p = self.safe_path(name)?;
        let parent = p.parent();
        let in_root = parent == Some(self.root.as_path());
        let nested = parent.is_some_and(|g| g.parent() == Some(self.root.as_path()) && self.is_group(g));
        if !(self.is_work_folder(&p) && (in_root || nested)) {
            return Err(err(ErrorKind::NotFound, "core.ws.noWorkFolder", vec![("name", name.to_string())]));
        }
        Ok(p)
    }

    // ---------- lectura ----------

    fn scan_files(&self, folder: &Path, skip: &BTreeSet<String>) -> (Vec<FileEntry>, f64, f64) {
        let created0 = std::fs::metadata(folder).map(|m| ctime(&m)).unwrap_or(0.0);
        let (mut created, mut last, mut last_work) = (created0, created0, None::<f64>);
        let mut files = Vec::new();
        let mut stack = vec![folder.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let top = dir == folder;
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if path.is_dir() {
                    if !name.starts_with('.') && !(top && skip.contains(&name)) {
                        stack.push(path);
                    }
                    continue;
                }
                let Ok(meta) = std::fs::metadata(&path) else { continue };
                let m = mtime(&meta);
                created = created.min(m);
                last = last.max(m);
                if !(top && name == README) {
                    last_work = Some(last_work.map_or(m, |l: f64| l.max(m)));
                }
                let ext = suffix(&name);
                let kind = self.wf.file_kind(&name).map(String::from);
                files.push(FileEntry {
                    path: self.rel(&path),
                    cat: self.wf.file_category(&name).id.clone(),
                    kind,
                    text: TEXT_EXT.contains(&ext.as_str()) && meta.len() <= MAX_TEXT_BYTES,
                    media: MEDIA_TYPES
                        .iter()
                        .find(|(e, _)| *e == ext)
                        .map(|(_, m)| *m)
                        .filter(|_| meta.len() <= MAX_MEDIA_BYTES),
                    size: meta.len(),
                    mtime: m,
                    name,
                });
            }
        }
        files.sort_by_key(|f| f.path.to_lowercase());
        (files, created, last_work.unwrap_or(last))
    }

    fn children(&self, dir: &Path) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| self.is_work_folder(p)).collect())
            .unwrap_or_default();
        out.sort();
        out
    }

    /// Un item o un grupo, sin `dupes` (eso sale de comparar con los demás: `list_items`).
    pub fn build_item(&self, folder: &Path) -> Result<Item> {
        let group = folder.parent() == Some(self.root.as_path()) && self.is_group(folder);
        let members: Vec<String> = if group {
            self.children(folder)
                .iter()
                .map(|c| c.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        } else {
            vec![]
        };
        let (files, created, last) = self.scan_files(folder, &members.iter().cloned().collect());
        let readme = folder.join(README);
        let has_readme = readme.exists();
        let keys = &self.wf.fields;
        let (mut meta, mut heading, mut deploy, mut tasks, mut gaps) =
            (Meta::new(), String::new(), String::new(), vec![], vec![]);
        if has_readme {
            let (m, body) = frontmatter::split(&frontmatter::read(&readme)?);
            heading = py::splitlines(&body, false)
                .iter()
                .find(|l| l.starts_with("# "))
                .map(|l| py::strip(&l[2..]).to_string())
                .unwrap_or_default();
            let kws: Vec<&str> = self.wf.deploy_keywords.iter().map(String::as_str).collect();
            deploy = text::extract_section(&body, &kws);
            tasks = self.tasks.parse(&body);
            let planned = m
                .get(&keys.status)
                .and_then(Value::as_text)
                .and_then(|s| self.wf.state(s))
                .is_some_and(|s| s.category == StateCategory::Planned);
            if !group && planned {
                gaps = text::plan_gaps(&self.wf, &body);
            }
            meta = m;
        }
        let folder_name = folder.file_name().unwrap().to_string_lossy().into_owned();
        let mut ids = if group { vec![] } else { self.wf.item_ids_in_folder(&folder_name) };
        if !group {
            match meta.get(&keys.item_ids) {
                Some(Value::Text(t)) if !t.is_empty() => ids = vec![t.clone()],
                Some(Value::List(l)) if !l.is_empty() => ids = l.clone(),
                _ => {}
            }
        }
        let name = self.rel(folder);
        let text_of = |k: &str| meta.get(k).and_then(Value::as_text).unwrap_or("").to_string();
        let title = Some(text_of(&keys.title)).filter(|t| !t.is_empty()).unwrap_or(heading);
        let item_type = Some(text_of(&keys.kind))
            .filter(|t| self.wf.types.iter().any(|ty| ty.id == *t))
            .unwrap_or_default();
        let parent = folder.parent().unwrap();
        Ok(Item {
            group: if parent == self.root {
                String::new()
            } else {
                parent.file_name().unwrap().to_string_lossy().into_owned()
            },
            members: members.iter().map(|m| format!("{name}/{m}")).collect(),
            kind: if group { ItemKind::Group } else { ItemKind::Item },
            size: files.iter().map(|f| f.size).sum(),
            kinds: files.iter().filter_map(|f| f.kind.clone()).collect::<BTreeSet<_>>().into_iter().collect(),
            name,
            ids,
            title,
            item_type,
            meta,
            has_readme,
            deploy,
            tasks,
            plan_gaps: gaps,
            created,
            last,
            files,
            dupes: None,
        })
    }

    /// Items de la raíz, los grupos y los items de cada grupo (un solo nivel).
    pub fn list_items(&self) -> Result<Vec<Item>> {
        let mut items = Vec::new();
        for c in self.children(&self.root) {
            items.push(self.build_item(&c)?);
            if self.is_group(&c) {
                for g in self.children(&c) {
                    items.push(self.build_item(&g)?);
                }
            }
        }
        let mut by_id: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for it in &items {
            for id in &it.ids {
                by_id.entry(id).or_default().push(&it.name);
            }
        }
        let dupes: Vec<Vec<String>> = items
            .iter()
            .map(|it| {
                let set: BTreeSet<&str> = it
                    .ids
                    .iter()
                    .flat_map(|id| by_id[id.as_str()].iter().copied())
                    .filter(|n| *n != it.name)
                    .collect();
                set.into_iter().map(String::from).collect()
            })
            .collect();
        for (it, d) in items.iter_mut().zip(dupes) {
            it.dupes = Some(d);
        }
        Ok(items)
    }

    /// Huella barata del árbol: cambia si se agrega, borra, renombra o modifica algo.
    pub fn signature(&self) -> (u64, f64) {
        let (mut count, mut total) = (0u64, 0.0f64);
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(meta) = std::fs::metadata(&dir) else { continue };
            total += mtime(&meta);
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if !e.file_name().to_string_lossy().starts_with('.') {
                        stack.push(p);
                    }
                } else if let Ok(m) = std::fs::metadata(&p) {
                    total += mtime(&m);
                    count += 1;
                }
            }
        }
        (count, (total * 1000.0).round() / 1000.0)
    }

    /// Un archivo existente para leerlo (visor): además del chequeo de `safe_path`, la ruta real (con
    /// enlaces simbólicos y junctions resueltos) tiene que seguir dentro de la raíz.
    pub fn existing_file(&self, rel: &str) -> Result<PathBuf> {
        let p = self.safe_path(rel)?;
        if !p.is_file() {
            return Err(err(ErrorKind::NotFound, "core.ws.fileNotFound", vec![("path", rel.to_string())]));
        }
        let real = dunce::canonicalize(&p)?;
        if !real.starts_with(&self.root) {
            return Err(err(ErrorKind::Permission, "core.ws.outsideRoot", vec![("rel", rel.to_string())]));
        }
        Ok(real)
    }

    /// Texto de un archivo de la raíz (hasta `MAX_TEXT_BYTES`), decodificado como los README.
    pub fn read_file(&self, rel: &str) -> Result<String> {
        let p = self.existing_file(rel)?;
        if std::fs::metadata(&p)?.len() > MAX_TEXT_BYTES {
            return Err(CoreError::invalid("core.ws.tooLarge", vec![]));
        }
        Ok(frontmatter::read(&p)?)
    }

    /// Imagen o PDF de la raíz (hasta `MAX_MEDIA_BYTES`) en base64: viaja igual por el IPC de Tauri y por
    /// el HTTP del devserver, y la UI lo convierte en un `blob:` local.
    pub fn read_media(&self, rel: &str) -> Result<Media> {
        let p = self.safe_path(rel)?;
        let ext = suffix(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let Some(mime) = MEDIA_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m) else {
            return Err(err(ErrorKind::NotFound, "core.ws.fileNotFound", vec![("path", rel.to_string())]));
        };
        let p = self.existing_file(rel)?;
        if std::fs::metadata(&p)?.len() > MAX_MEDIA_BYTES {
            return Err(CoreError::invalid("core.ws.tooLarge", vec![]));
        }
        Ok(Media { mime, data: base64::engine::general_purpose::STANDARD.encode(std::fs::read(&p)?) })
    }

    pub fn get_meta(&self, name: &str) -> Result<Meta> {
        let readme = self.folder_of(name)?.join(README);
        if !readme.exists() {
            return Ok(Meta::new());
        }
        Ok(frontmatter::split(&frontmatter::read(&readme)?).0)
    }

    /// Lo que hay en la carpeta de eliminados (borrado desde la app o a mano), lo último primero.
    pub fn list_deleted(&self) -> Result<Vec<DeletedEntry>> {
        let bin = self.root.join(&self.wf.folders.deleted_dir);
        let Ok(rd) = std::fs::read_dir(&bin) else { return Ok(vec![]) };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let c = e.path();
            if !c.is_dir() {
                continue;
            }
            let (mut meta, mut title) = (Meta::new(), String::new());
            let readme = c.join(README);
            if readme.exists() {
                let (m, body) = frontmatter::split(&frontmatter::read(&readme)?);
                let heading = py::splitlines(&body, false)
                    .iter()
                    .find(|l| l.starts_with("# "))
                    .map(|l| py::strip(&l[2..]).to_string())
                    .unwrap_or_default();
                title = m
                    .get(&self.wf.fields.title)
                    .and_then(Value::as_text)
                    .filter(|t| !t.is_empty())
                    .map_or(heading, String::from);
                meta = m;
            }
            out.push(DeletedEntry {
                path: self.rel(&c),
                base: e.file_name().to_string_lossy().into_owned(),
                title,
                status: meta.get(&self.wf.fields.status).and_then(Value::as_text).unwrap_or("").to_string(),
                deleted: std::fs::metadata(&c).map(|m| mtime(&m)).unwrap_or(0.0),
                group: self.is_group(&c),
            });
        }
        out.sort_by(|a, b| b.deleted.total_cmp(&a.deleted));
        Ok(out)
    }
}
