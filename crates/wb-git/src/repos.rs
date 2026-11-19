//! Lo que se lee de cada repo sin recorrer la historia: ramas (`for-each-ref`), rama actual y archivos
//! sin commitear (`status`), remotes (de `.git/config`), último fetch, repos hermanos y la firma para
//! vigilar cambios.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::cmd::git;
use crate::{GitError, RepoConfig, Rules, Tip};

/// Archivos de `.git` que cambian con un fetch, commit, checkout, merge o cherry-pick (y las carpetas
/// de refs, que cambian al crear o borrar una rama).
pub const WATCH_FILES: &[&str] =
    &["FETCH_HEAD", "HEAD", "ORIG_HEAD", "logs/HEAD", "packed-refs", "index", "refs/heads", "refs/remotes"];

pub(crate) struct Group {
    pub name: String,
    pub nums: BTreeSet<String>,
    /// `local` o el remote → sha, en el orden de `for-each-ref`.
    pub locations: IndexMap<String, String>,
    pub tip: Tip,
}

pub(crate) struct Snapshot {
    pub groups: Vec<Group>,
    /// Ramas remotas (`origin/development`) → sha.
    pub refs: HashMap<String, String>,
    pub current: String,
    pub dirty: Vec<String>,
    pub last_fetch: Option<f64>,
}

/// Remotes del repo leyendo `.git/config` (sin lanzar `git remote`). `None` = no se pudo leer (worktree).
pub(crate) fn remotes(repo: &Path) -> Option<IndexMap<String, Option<String>>> {
    let cfg = std::fs::read(repo.join(".git").join("config")).ok()?;
    let cfg = String::from_utf8_lossy(&cfg);
    let mut out = IndexMap::new();
    let mut current: Option<String> = None;
    for line in cfg.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            current = l
                .strip_prefix('[')
                .and_then(|r| r.strip_suffix(']'))
                .map(str::trim)
                .and_then(|r| r.strip_prefix("remote"))
                .filter(|r| r.starts_with(char::is_whitespace))
                .map(str::trim)
                .and_then(|r| r.strip_prefix('"')?.strip_suffix('"'))
                .map(String::from);
            if let Some(name) = &current {
                out.entry(name.clone()).or_insert(None);
            }
        } else if let Some(name) = &current
            && let Some((k, v)) = l.split_once('=')
            && k.trim().eq_ignore_ascii_case("url")
        {
            let v = v.trim().trim_matches('"').to_string();
            out.insert(name.clone(), Some(v));
        }
    }
    Some(out)
}

pub(crate) fn snapshot(repo: &Path, rules: &Rules) -> Result<Snapshot, GitError> {
    let remotes = remotes(repo);
    let fmt = "--format=%(refname)%09%(objectname)%09%(committerdate:unix)%09%(authorname)%09%(subject)";
    let out = git(repo, &["for-each-ref", fmt, "refs/heads", "refs/remotes"], None, true)?.text();
    let mut groups: IndexMap<String, Group> = IndexMap::new();
    let mut refs = HashMap::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 5 {
            continue;
        }
        let (full, sha, ts, author, subject) = (parts[0], parts[1], parts[2], parts[3], parts[4]);
        if full.ends_with("/HEAD") {
            continue;
        }
        let (loc, name) = if let Some(short) = full.strip_prefix("refs/remotes/") {
            // Para resolver los entornos (origin/development…) sin otro comando.
            refs.insert(short.to_string(), sha.to_string());
            let (remote, name) = short.split_once('/').unwrap_or((short, ""));
            if remotes.as_ref().is_some_and(|r| !r.contains_key(remote)) {
                continue;
            }
            (remote.to_string(), name.to_string())
        } else {
            ("local".to_string(), full.strip_prefix("refs/heads/").unwrap_or(full).to_string())
        };
        let nums = rules.ids_in_name(&name);
        // `promote/…` son las ramas que arma la promoción: copias de commits, no trabajo de un item.
        if nums.is_empty() || name.to_lowercase().starts_with(&rules.promote_prefix.to_lowercase()) {
            continue;
        }
        let ts: i64 = ts.parse().unwrap_or(0);
        // Windows: origin/Add/x y origin/add/x colisionan; se agrupa sin distinguir mayúsculas.
        let g = groups.entry(name.to_lowercase()).or_insert_with(|| Group {
            name: name.clone(),
            nums,
            locations: IndexMap::new(),
            tip: Tip {
                sha: String::new(),
                ts: i64::MIN,
                author: String::new(),
                subject: String::new(),
                loc: String::new(),
            },
        });
        g.locations.insert(loc.clone(), sha.to_string());
        if ts > g.tip.ts {
            g.tip = Tip {
                sha: sha.to_string(),
                ts,
                author: author.to_string(),
                subject: subject.to_string(),
                loc,
            };
        }
    }
    let st = git(repo, &["status", "--porcelain=v1", "-b"], None, true)?.text();
    let lines: Vec<&str> = st.lines().collect();
    let current = match lines.first() {
        Some(l) if l.starts_with("## ") => l[3..].split("...").next().unwrap_or("").to_string(),
        _ => String::new(),
    };
    let dirty = lines.iter().skip(1).map(|l| l.get(3..).unwrap_or("").to_string()).collect();
    Ok(Snapshot {
        groups: groups.into_values().collect(),
        refs,
        current,
        dirty,
        last_fetch: last_fetch(repo),
    })
}

/// Fecha del último fetch (mtime de `FETCH_HEAD`), en segundos.
pub(crate) fn last_fetch(repo: &Path) -> Option<f64> {
    let m = std::fs::metadata(repo.join(".git").join("FETCH_HEAD")).ok()?.modified().ok()?;
    Some(m.duration_since(UNIX_EPOCH).ok()?.as_secs_f64())
}

/// Ramas remotas del repo (`origin/main`…).
fn remote_refs(repo: &Path) -> Result<HashSet<String>, GitError> {
    let out = git(repo, &["for-each-ref", "--format=%(refname)", "refs/remotes"], None, true)?.text();
    Ok(out.lines().filter_map(|l| l.strip_prefix("refs/remotes/")).map(String::from).collect())
}

/// Entornos de un repo: para cada uno, la primera rama candidata que existe.
pub fn default_envs(
    repo: &Path,
    candidates: &IndexMap<String, Vec<String>>,
) -> Result<IndexMap<String, String>, GitError> {
    let refs = remote_refs(repo)?;
    Ok(candidates
        .iter()
        .filter_map(|(env, cands)| cands.iter().find(|c| refs.contains(*c)).map(|c| (env.clone(), c.clone())))
        .collect())
}

/// Repos hermanos de la carpeta de trabajos (`…/proyecto/_work` → repos en `…/proyecto/`). Un repo sin
/// ninguna rama de entorno no es del flujo (un vault, por ejemplo) y no cuenta.
pub fn detect_repos(work_root: &Path, candidates: &IndexMap<String, Vec<String>>) -> Vec<RepoConfig> {
    let root = dunce::canonicalize(work_root).unwrap_or_else(|_| work_root.to_path_buf());
    let Some(base) = root.parent() else { return vec![] };
    let Ok(entries) = std::fs::read_dir(base) else { return vec![] };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort_by_key(|d| d.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
    dirs.into_iter()
        // `.git` como archivo = worktree de otro repo: comparte sus ramas, sumarlo duplicaría todo.
        .filter(|d| d.join(".git").is_dir())
        .filter_map(|d| {
            let envs = default_envs(&d, candidates).ok().filter(|e| !e.is_empty())?;
            let name = d.file_name()?.to_string_lossy().into_owned();
            Some(RepoConfig { name, path: d.display().to_string(), envs, compare: true })
        })
        .collect()
}

/// ¿Es la carpeta de un repo git?
pub fn is_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Dónde se abre un PR del repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Forge {
    Azure,
    Github,
    Gitlab,
}

/// URL web de un remote y su forja: Azure DevOps (`…/_git/…`, como la v1), GitHub y GitLab (https o
/// `git@host:dueño/repo`). `None` si no se puede derivar.
pub fn web_url(remote_url: &str) -> Option<(Forge, String)> {
    let url = remote_url.trim();
    let https = if let Some(rest) = url.strip_prefix("https://") {
        // Sin usuario embebido.
        let rest = match rest.split_once('@') {
            Some((user, after)) if !user.contains('/') => after,
            _ => rest,
        };
        format!("https://{rest}")
    } else {
        let (host, path) = url.strip_prefix("git@")?.split_once(':')?;
        if host.contains("azure") {
            return None;
        }
        format!("https://{host}/{path}")
    };
    let https = https.strip_suffix(".git").unwrap_or(&https).trim_end_matches('/').to_string();
    let host = https["https://".len()..].split('/').next().unwrap_or("").to_lowercase();
    let forge = if https.contains("/_git/") {
        Forge::Azure
    } else if host.contains("github") {
        Forge::Github
    } else if host.contains("gitlab") {
        Forge::Gitlab
    } else {
        return None;
    };
    Some((forge, https))
}

/// URL web de un remote del repo.
pub fn remote_web_url(repo: &Path, remote: &str) -> Option<(Forge, String)> {
    let url = remotes(repo)?.get(remote)?.clone()?;
    web_url(&url)
}

/// Firma barata de los repos (mtime y tamaño de los archivos de `.git` que cambian): si cambia, hay que
/// releer git.
pub fn watch_signature(repos: &[RepoConfig]) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    for r in repos {
        r.path.hash(&mut h);
        for f in WATCH_FILES {
            if let Ok(m) = std::fs::metadata(Path::new(&r.path).join(".git").join(f)) {
                m.len().hash(&mut h);
                m.modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .hash(&mut h);
            } else {
                0u8.hash(&mut h);
            }
        }
    }
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_urls() {
        let w = web_url;
        assert_eq!(
            w("https://user@dev.azure.com/org/proj/_git/app.git"),
            Some((Forge::Azure, "https://dev.azure.com/org/proj/_git/app".into()))
        );
        assert_eq!(
            w("https://github.com/acme/app.git"),
            Some((Forge::Github, "https://github.com/acme/app".into()))
        );
        assert_eq!(
            w("git@github.com:acme/app.git"),
            Some((Forge::Github, "https://github.com/acme/app".into()))
        );
        assert_eq!(
            w("https://gitlab.example.com/g/app"),
            Some((Forge::Gitlab, "https://gitlab.example.com/g/app".into()))
        );
        assert_eq!(w("git@ssh.dev.azure.com:v3/org/proj/app"), None);
        assert_eq!(w("http://server/_git/app"), None);
        assert_eq!(w("https://example.com/acme/app.git"), None);
    }
}
