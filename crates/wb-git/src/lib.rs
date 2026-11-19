//! Panel Git por item. **Solo lectura**: `for-each-ref`, `rev-list`, `cherry`, `merge-base`, `diff`,
//! `diff-tree`, `log`, `patch-id` y `status --no-optional-locks` (la lista se controla en cada llamada, ver
//! [`cmd::READ_ONLY`]). Nunca fetch, checkout ni nada que escriba. La promoción solo devuelve datos para
//! armar comandos que corre el usuario.
//!
//! Para cada item busca las ramas cuyo nombre contiene su id (una rama puede traer varios:
//! `Adds/568994-573877`) en los repos configurados y dice si su trabajo ya está en cada entorno. Se evalúan
//! los commits **propios** de la rama, no todo lo que arrastra: la línea first-parent de la rama hasta
//! tocar la línea principal de algún entorno.
//!
//! - `merged`: los commits propios son alcanzables desde el entorno.
//! - `equivalent`: cada commit propio tiene un equivalente (mismo patch, como `git cherry`, o un
//!   `cherry-pick -x` cuyo trailer lo nombra, aunque haya habido conflicto).
//! - `content`: los archivos que tocó la rama están idénticos en el entorno (squash).
//! - `mentioned`: un commit del entorno menciona el item en el mensaje.
//! - `pending`: falta; `ahead` = commits propios que no están.
//!
//! El estado de cada rama se calcula sobre un grafo en memoria ([`graph`]); lo que el grafo no responde
//! va a git en lote (un `diff-tree --stdin` para los archivos, patch-ids para la equivalencia), todo
//! cacheado por sha en disco (`git_cache.json`). Los repos se leen en paralelo.

pub mod cmd;
mod graph;
mod promotion;
mod repos;
mod rules;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use cmd::git;
use graph::RepoGraph;
pub use promotion::{Flag, PromoBranch, PromoRepo, PromoRepoResult, Promotion, Row};
pub use repos::{
    Forge, WATCH_FILES, default_envs, detect_repos, is_repo, remote_web_url, watch_signature, web_url,
};
pub use rules::{Rules, cp_trailer};

/// Ventana (días) para buscar menciones y cherry-picks en los mensajes de cada entorno.
pub const LOG_DAYS: i64 = 240;
/// Repos en paralelo.
const REPO_WORKERS: usize = 4;
/// Entradas por tabla del caché en disco.
const CACHE_DISK_MAX: usize = 20_000;
/// Versión del cálculo: un caché de otra versión se descarta.
const ALGO: &str = "wb-git-1";

/// Commits de cherry-pick -x de un entorno y los originales que nombran.
pub(crate) type CpMap = Arc<Vec<(String, Vec<String>)>>;
/// Una rama a evaluar: su tip y los entornos que mencionan sus items.
type BranchKey = (String, Vec<String>);
type ShaCache<V> = Mutex<HashMap<(String, String), V>>;

/// Texto traducible: una clave de `locales/*.json` y sus parámetros. El backend no arma texto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Msg {
    pub code: String,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

impl Msg {
    pub fn new(code: &str, params: &[(&str, &str)]) -> Self {
        Self {
            code: code.into(),
            params: params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }
}

/// Error de git (o de la configuración de un repo), con código traducible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError {
    pub code: &'static str,
    pub params: Vec<(&'static str, String)>,
}

impl GitError {
    pub fn new(code: &'static str, params: &[(&'static str, &str)]) -> Self {
        Self { code, params: params.iter().map(|(k, v)| (*k, v.to_string())).collect() }
    }

    pub fn msg(&self) -> Msg {
        Msg {
            code: self.code.into(),
            params: self.params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
        }
    }
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code)?;
        for (k, v) in &self.params {
            write!(f, " {k}={v}")?;
        }
        Ok(())
    }
}

impl std::error::Error for GitError {}

/// Un repo que se revisa: nombre, carpeta y la rama remota de cada entorno (en el orden del flujo: la
/// promoción a un entorno sale del anterior).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoConfig {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub envs: IndexMap<String, String>,
    /// Se compara con los otros repos (alerta de repos desparejos en un entorno).
    #[serde(default = "yes")]
    pub compare: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tip {
    pub sha: String,
    pub ts: i64,
    pub author: String,
    pub subject: String,
    /// `local` o el remote.
    pub loc: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Merged,
    Equivalent,
    Content,
    Mentioned,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvState {
    /// Commits propios que faltan (`None`: no se pudieron aislar los commits propios).
    pub ahead: Option<usize>,
    pub state: State,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BranchStatus {
    own: usize,
    files: usize,
    merged_into: Option<String>,
    envs: IndexMap<String, EnvState>,
}

/// Una rama de un item en un repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub repo: String,
    pub name: String,
    pub locations: Vec<String>,
    pub tip: Tip,
    pub envs: IndexMap<String, EnvState>,
    /// Commits propios y archivos que tocan.
    pub own: usize,
    pub files: usize,
    /// Primer entorno (en orden) donde el trabajo ya está.
    pub merged_into: Option<String>,
    /// Ids de items en el nombre de la rama.
    pub ids: Vec<String>,
    /// Es la rama actual del repo.
    pub current: bool,
}

/// El commit más reciente de un entorno que menciona un item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mention {
    pub sha: String,
    pub ts: i64,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub name: String,
    pub path: String,
    pub envs: IndexMap<String, String>,
    /// Entornos configurados cuya rama no existe (¿falta fetch?).
    pub missing_envs: Vec<String>,
    pub current: String,
    /// Archivos sin commitear (hasta 200) y cuántos son.
    pub dirty: Vec<String>,
    pub dirty_count: usize,
    pub last_fetch: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RepoResult {
    Ok(RepoInfo),
    Err { name: String, error: Msg },
}

/// Vista general: repos, ramas por id de item y menciones por id → repo → entorno.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub repos: Vec<RepoResult>,
    pub by_item: IndexMap<String, Vec<BranchInfo>>,
    pub mentions: IndexMap<String, IndexMap<String, IndexMap<String, Mention>>>,
    pub computed_at: f64,
    pub seconds: f64,
}

/// Diff de los commits propios de una rama (para revisarla con otro modelo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchDiff {
    pub base: String,
    pub tip: String,
    pub commits: usize,
    pub stat: String,
    pub diff: String,
    /// El diff se recortó a `max_chars`.
    pub cut: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct DiskCache {
    #[serde(default)]
    algo: String,
    /// Estado de una rama por (repo, tip, shas de los entornos, entornos que la mencionan).
    #[serde(default)]
    status: IndexMap<String, BranchStatus>,
    /// Archivos entre dos commits.
    #[serde(default)]
    files: IndexMap<String, Vec<String>>,
    /// ¿Los archivos de la rama están iguales en el entorno?
    #[serde(default)]
    same: IndexMap<String, bool>,
    /// patch-id por repo y sha ("" = sin cambios).
    #[serde(default)]
    pids: HashMap<String, HashMap<String, String>>,
    #[serde(skip)]
    dirty: bool,
}

/// El panel: caché en disco, grafos en memoria y la hora de referencia (fija en los tests).
pub struct GitPanel {
    cache_file: Option<PathBuf>,
    cache: Mutex<DiskCache>,
    graphs: Mutex<HashMap<String, Arc<Mutex<RepoGraph>>>>,
    fp: ShaCache<Arc<HashSet<String>>>,
    cp: ShaCache<CpMap>,
    now: Option<i64>,
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

impl GitPanel {
    /// `cache_file`: `<config>/git_cache.json` (o `None`, sin caché en disco).
    pub fn new(cache_file: Option<PathBuf>) -> Self {
        let mut cache: DiskCache = cache_file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if cache.algo != ALGO {
            cache = DiskCache { algo: ALGO.into(), ..Default::default() };
        }
        Self {
            cache_file,
            cache: Mutex::new(cache),
            graphs: Mutex::new(HashMap::new()),
            fp: Mutex::new(HashMap::new()),
            cp: Mutex::new(HashMap::new()),
            now: None,
        }
    }

    /// Fija la hora de referencia (segundos) para la ventana de [`LOG_DAYS`].
    pub fn with_now(mut self, now: i64) -> Self {
        self.now = Some(now);
        self
    }

    pub(crate) fn since(&self) -> i64 {
        self.now.unwrap_or_else(|| unix_now() as i64) - LOG_DAYS * 86_400
    }

    fn save_cache(&self) {
        let Some(file) = &self.cache_file else { return };
        let raw = {
            let mut c = self.cache.lock().expect("git cache");
            if !c.dirty {
                return;
            }
            c.dirty = false;
            // Tope para que no crezca sin fin (los patch-ids, por repo, no se recortan).
            trim(&mut c.status);
            trim(&mut c.files);
            trim(&mut c.same);
            match serde_json::to_vec(&*c) {
                Ok(v) => v,
                Err(_) => return,
            }
        };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("json.tmp");
        if std::fs::write(&tmp, raw).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }

    fn graph(&self, repo: &Path) -> Arc<Mutex<RepoGraph>> {
        let key = repo.display().to_string();
        self.graphs
            .lock()
            .expect("graphs")
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(RepoGraph::new(repo.to_path_buf()))))
            .clone()
    }

    /// Ramas y estado por entorno de los items `ids` en `repos`.
    pub fn overview(&self, repos: &[RepoConfig], ids: &BTreeSet<String>, rules: &Rules) -> Overview {
        let t0 = Instant::now();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let slots: Vec<Mutex<Option<RepoOverview>>> = (0..repos.len()).map(|_| Mutex::new(None)).collect();
        std::thread::scope(|s| {
            for _ in 0..REPO_WORKERS.min(repos.len()) {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        if i >= repos.len() {
                            break;
                        }
                        let out = self.overview_repo(&repos[i], ids, rules);
                        *slots[i].lock().expect("slot") = Some(out);
                    }
                });
            }
        });
        let results: Vec<Option<RepoOverview>> =
            slots.into_iter().map(|s| s.into_inner().expect("slot")).collect();
        let mut out = Overview {
            repos: vec![],
            by_item: IndexMap::new(),
            mentions: IndexMap::new(),
            computed_at: 0.0,
            seconds: 0.0,
        };
        for (r, res) in repos.iter().zip(results) {
            let Some(res) = res else { continue };
            out.repos.push(res.info);
            for (nums, entry) in res.entries {
                for id in nums {
                    out.by_item.entry(id).or_default().push(entry.clone());
                }
            }
            for (id, per_env) in res.mentions {
                out.mentions.entry(id).or_default().insert(r.name.clone(), per_env);
            }
        }
        for branches in out.by_item.values_mut() {
            branches.sort_by(|a, b| {
                a.repo.to_lowercase().cmp(&b.repo.to_lowercase()).then(b.tip.ts.cmp(&a.tip.ts))
            });
        }
        self.save_cache();
        out.computed_at = unix_now();
        out.seconds = (t0.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        out
    }

    fn overview_repo(&self, r: &RepoConfig, wanted: &BTreeSet<String>, rules: &Rules) -> RepoOverview {
        let path = Path::new(&r.path);
        let fail = |e: GitError| RepoOverview {
            info: RepoResult::Err { name: r.name.clone(), error: e.msg() },
            entries: vec![],
            mentions: IndexMap::new(),
        };
        if !is_repo(path) {
            return fail(GitError::new("git.notRepo", &[("path", &r.path)]));
        }
        self.overview_repo_inner(r, path, wanted, rules).unwrap_or_else(fail)
    }

    fn overview_repo_inner(
        &self,
        r: &RepoConfig,
        path: &Path,
        wanted: &BTreeSet<String>,
        rules: &Rules,
    ) -> Result<RepoOverview, GitError> {
        let snap = repos::snapshot(path, rules)?;
        let env_shas: IndexMap<String, String> = r
            .envs
            .iter()
            .filter_map(|(env, rf)| snap.refs.get(rf).map(|sha| (env.clone(), sha.clone())))
            .collect();
        let mut hits: Vec<&repos::Group> =
            snap.groups.iter().filter(|g| g.nums.iter().any(|n| wanted.contains(n))).collect();
        hits.sort_by_key(|g| std::cmp::Reverse(g.tip.ts));
        let graph = self.graph(path);
        let mut graph = graph.lock().expect("graph");
        let tips: Vec<&str> = env_shas
            .values()
            .map(String::as_str)
            .chain(hits.iter().flat_map(|g| g.locations.values().map(String::as_str)))
            .collect();
        graph.update(tips)?;
        let since = self.since();
        let mut mentions: IndexMap<String, IndexMap<String, Mention>> = IndexMap::new();
        for (env, sha) in &env_shas {
            for (id, c) in graph.mentions(sha, wanted, rules, since) {
                mentions.entry(id).or_default().insert(env.clone(), c);
            }
        }
        let picked: HashMap<String, HashSet<String>> =
            env_shas.keys().map(|env| (env.clone(), graph.picked_into(&env_shas, env, since))).collect();
        let mut items: Vec<(&repos::Group, BTreeSet<String>, BranchKey)> = vec![];
        for g in &hits {
            let nums: BTreeSet<String> = g.nums.intersection(wanted).cloned().collect();
            let envs: BTreeSet<String> =
                nums.iter().filter_map(|id| mentions.get(id)).flat_map(|m| m.keys().cloned()).collect();
            items.push((g, nums, (g.tip.sha.clone(), envs.into_iter().collect())));
        }
        let mut unique: Vec<(String, Vec<String>)> = vec![];
        for (_, _, it) in &items {
            if !unique.contains(it) {
                unique.push(it.clone());
            }
        }
        let statuses = self.statuses(path, &mut graph, &unique, &env_shas, &picked)?;
        drop(graph);
        let entries = items
            .into_iter()
            .map(|(g, nums, it)| {
                let st = &statuses[&it];
                let mut locations: Vec<String> = g.locations.keys().cloned().collect();
                locations.sort();
                let entry = BranchInfo {
                    repo: r.name.clone(),
                    name: g.name.clone(),
                    locations,
                    tip: g.tip.clone(),
                    envs: st.envs.clone(),
                    own: st.own,
                    files: st.files,
                    merged_into: st.merged_into.clone(),
                    ids: g.nums.iter().cloned().collect(),
                    current: snap.current.to_lowercase() == g.name.to_lowercase(),
                };
                (nums, entry)
            })
            .collect();
        let info = RepoInfo {
            name: r.name.clone(),
            path: path.display().to_string(),
            envs: r.envs.clone(),
            missing_envs: r.envs.keys().filter(|e| !env_shas.contains_key(*e)).cloned().collect(),
            current: snap.current.clone(),
            dirty: snap.dirty.iter().take(200).cloned().collect(),
            dirty_count: snap.dirty.len(),
            last_fetch: snap.last_fetch,
        };
        Ok(RepoOverview { info: RepoResult::Ok(info), entries, mentions })
    }

    /// patch-ids de estos commits; solo se le piden a git los que no están en el caché.
    fn patch_ids(&self, repo: &Path, shas: &BTreeSet<String>) -> Result<HashMap<String, String>, GitError> {
        let key = repo.display().to_string();
        let missing: Vec<String> = {
            let c = self.cache.lock().expect("git cache");
            let store = c.pids.get(&key);
            shas.iter().filter(|s| !store.is_some_and(|st| st.contains_key(*s))).cloned().collect()
        };
        if !missing.is_empty() {
            let stdin = missing.join("\n") + "\n";
            let patches = git(
                repo,
                &[
                    "log",
                    "--no-walk=unsorted",
                    "--stdin",
                    "-p",
                    "--no-renames",
                    "--no-color",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--format=commit %H",
                ],
                Some(stdin.as_bytes()),
                true,
            )?
            .stdout;
            let mut got = HashMap::new();
            if patches.iter().any(|b| !b.is_ascii_whitespace()) {
                let out = git(repo, &["patch-id"], Some(&patches), true)?.text();
                for line in out.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() == 2 {
                        got.insert(parts[1].to_string(), parts[0].to_string());
                    }
                }
            }
            let mut c = self.cache.lock().expect("git cache");
            let store = c.pids.entry(key.clone()).or_default();
            for s in missing {
                let pid = got.get(&s).cloned().unwrap_or_default();
                store.insert(s, pid);
            }
            c.dirty = true;
        }
        let c = self.cache.lock().expect("git cache");
        let store = c.pids.get(&key);
        Ok(shas
            .iter()
            .map(|s| (s.clone(), store.and_then(|st| st.get(s)).cloned().unwrap_or_default()))
            .collect())
    }

    /// Archivos distintos entre cada par, con caché.
    fn files_between(
        &self,
        repo: &Path,
        pairs: &BTreeSet<(String, String)>,
    ) -> Result<HashMap<(String, String), Vec<String>>, GitError> {
        let key = |a: &str, b: &str| format!("{}|{a}|{b}", repo.display());
        let mut out = HashMap::new();
        let mut missing = vec![];
        {
            let c = self.cache.lock().expect("git cache");
            for (a, b) in pairs {
                match c.files.get(&key(a, b)) {
                    Some(v) => {
                        out.insert((a.clone(), b.clone()), v.clone());
                    }
                    None => missing.push((a.clone(), b.clone())),
                }
            }
        }
        if !missing.is_empty() {
            let new = diff_tree(repo, &missing)?;
            let mut c = self.cache.lock().expect("git cache");
            for p in missing {
                let files: Vec<String> = new.get(&p).map(|s| s.iter().cloned().collect()).unwrap_or_default();
                c.files.insert(key(&p.0, &p.1), files.clone());
                out.insert(p, files);
            }
            c.dirty = true;
        }
        Ok(out)
    }

    /// Estado de cada rama `(tip, entornos que mencionan el item)` en cada entorno.
    fn statuses(
        &self,
        repo: &Path,
        graph: &mut RepoGraph,
        items: &[(String, Vec<String>)],
        env_shas: &IndexMap<String, String>,
        picked: &HashMap<String, HashSet<String>>,
    ) -> Result<HashMap<(String, Vec<String>), BranchStatus>, GitError> {
        struct Work {
            item: (String, Vec<String>),
            key: String,
            tip: String,
            own: Vec<String>,
            base: String,
            files: BTreeSet<String>,
            envs: IndexMap<String, EnvState>,
            reach: Option<HashSet<u32>>,
        }
        let mut results = HashMap::new();
        let mut work: Vec<Work> = vec![];
        let env_part: Vec<String> = {
            let mut v: Vec<String> = env_shas.iter().map(|(k, v)| format!("{k}={v}")).collect();
            v.sort();
            v
        };
        for (tip, mentioned) in items {
            let key = [repo.display().to_string(), tip.clone()]
                .into_iter()
                .chain(env_part.iter().cloned())
                .chain(mentioned.iter().cloned())
                .collect::<Vec<_>>()
                .join("|");
            if let Some(hit) = self.cache.lock().expect("git cache").status.get(&key) {
                results.insert((tip.clone(), mentioned.clone()), hit.clone());
                continue;
            }
            let own_all = graph.own_commits(tip, env_shas);
            let base = match own_all.first() {
                Some((first, _)) => {
                    let p = graph.parent(first);
                    if p.is_empty() { first.clone() } else { p }
                }
                None => String::new(),
            };
            work.push(Work {
                item: (tip.clone(), mentioned.clone()),
                key,
                tip: tip.clone(),
                own: own_all.iter().filter(|(_, m)| !m).map(|(s, _)| s.clone()).collect(),
                base,
                files: BTreeSet::new(),
                envs: IndexMap::new(),
                reach: None,
            });
        }
        if work.is_empty() {
            return Ok(results);
        }

        // 1) Archivos que tocó cada rama (base → tip).
        let pairs: BTreeSet<(String, String)> =
            work.iter().filter(|w| !w.own.is_empty()).map(|w| (w.tip.clone(), w.base.clone())).collect();
        let files = self.files_between(repo, &pairs)?;
        // 2) Qué commits propios faltan en cada entorno; los que no llegaron por cherry-pick -x se comparan
        //    por patch-id contra lo que el entorno tiene y la rama no (lo mismo que `git cherry`).
        let mut need_pid: BTreeSet<String> = BTreeSet::new();
        let mut cherry: Vec<(usize, String, Vec<String>, Vec<String>)> = vec![];
        for (wi, w) in work.iter_mut().enumerate() {
            if !w.own.is_empty() {
                w.files = files
                    .get(&(w.tip.clone(), w.base.clone()))
                    .map(|v| v.iter().cloned().collect())
                    .unwrap_or_default();
            }
            for (env, sha) in env_shas {
                let Some(env_id) = graph.id(sha) else { continue };
                let anc = graph.ancestors(env_id);
                let inside = |s: &str| graph.id(s).is_some_and(|i| anc.contains(&i));
                if w.own.is_empty() {
                    // Fast-forward o rama vacía: solo se puede decir si el tip está en el entorno.
                    let tip_in = inside(&w.tip);
                    let state = if tip_in {
                        State::Merged
                    } else if w.item.1.contains(env) {
                        State::Mentioned
                    } else {
                        State::Pending
                    };
                    w.envs
                        .insert(env.clone(), EnvState { ahead: if tip_in { Some(0) } else { None }, state });
                    continue;
                }
                let not_in: Vec<String> = w.own.iter().filter(|c| !inside(c)).cloned().collect();
                if not_in.is_empty() {
                    w.envs.insert(env.clone(), EnvState { ahead: Some(0), state: State::Merged });
                    continue;
                }
                let cand: Vec<String> = not_in.into_iter().filter(|c| !cp_hit(c, &picked[env])).collect();
                if cand.is_empty() {
                    w.envs.insert(env.clone(), EnvState { ahead: Some(0), state: State::Equivalent });
                    continue;
                }
                if w.reach.is_none() {
                    w.reach = Some(graph.id(&w.tip).map(|i| graph.reachable(i)).unwrap_or_default());
                }
                let reach = w.reach.as_ref().expect("reach");
                let upstream: Vec<String> = anc
                    .iter()
                    .copied()
                    .filter(|i| !reach.contains(i) && !graph.is_merge(*i))
                    .map(|i| graph.node(i).sha.clone())
                    .collect();
                need_pid.extend(cand.iter().cloned());
                need_pid.extend(upstream.iter().cloned());
                // Lugar reservado para mantener el orden de los entornos.
                w.envs.insert(env.clone(), EnvState { ahead: None, state: State::Pending });
                cherry.push((wi, env.clone(), cand, upstream));
            }
        }
        let pids = if need_pid.is_empty() { HashMap::new() } else { self.patch_ids(repo, &need_pid)? };
        let mut content_q: Vec<(usize, String)> = vec![];
        for (wi, env, cand, upstream) in cherry {
            let env_pids: HashSet<&String> = upstream.iter().map(|s| &pids[s]).collect();
            let truly = cand.iter().filter(|c| !env_pids.contains(&pids[*c])).count();
            let w = &mut work[wi];
            let mut st = EnvState {
                ahead: Some(truly),
                state: if truly > 0 { State::Pending } else { State::Equivalent },
            };
            if truly > 0 && !w.files.is_empty() {
                content_q.push((wi, env.clone()));
            } else if truly > 0 && w.item.1.contains(&env) {
                st.state = State::Mentioned;
            }
            w.envs.insert(env, st);
        }
        // 3) ¿Los archivos de la rama ya están iguales en el entorno? (squash o merge manual)
        if !content_q.is_empty() {
            let keys: BTreeSet<(String, String, String)> = content_q
                .iter()
                .map(|(wi, env)| (work[*wi].tip.clone(), env_shas[env].clone(), work[*wi].base.clone()))
                .collect();
            let files_by_tip: HashMap<String, BTreeSet<String>> =
                content_q.iter().map(|(wi, _)| (work[*wi].tip.clone(), work[*wi].files.clone())).collect();
            let same = self.same_content(repo, &keys, &files_by_tip)?;
            for (wi, env) in content_q {
                let w = &mut work[wi];
                let k = (w.tip.clone(), env_shas[&env].clone(), w.base.clone());
                let mentioned = w.item.1.contains(&env);
                let st = w.envs.get_mut(&env).expect("env state");
                if same[&k] {
                    st.state = State::Content;
                } else if mentioned {
                    st.state = State::Mentioned;
                }
            }
        }
        let mut c = self.cache.lock().expect("git cache");
        for w in work {
            let merged_into =
                env_shas.keys().find(|e| w.envs.get(*e).is_some_and(|s| s.state != State::Pending)).cloned();
            let res = BranchStatus { own: w.own.len(), files: w.files.len(), merged_into, envs: w.envs };
            c.status.insert(w.key, res.clone());
            c.dirty = true;
            results.insert(w.item, res);
        }
        Ok(results)
    }

    /// `{(tip, env, base): true si los archivos que tocó la rama están iguales en el entorno}`.
    fn same_content(
        &self,
        repo: &Path,
        keys: &BTreeSet<(String, String, String)>,
        files_by_tip: &HashMap<String, BTreeSet<String>>,
    ) -> Result<HashMap<(String, String, String), bool>, GitError> {
        let key = |k: &(String, String, String)| format!("{}|{}|{}|{}", repo.display(), k.0, k.1, k.2);
        let mut out = HashMap::new();
        let mut missing = vec![];
        {
            let c = self.cache.lock().expect("git cache");
            for k in keys {
                match c.same.get(&key(k)) {
                    Some(v) => {
                        out.insert(k.clone(), *v);
                    }
                    None => missing.push(k.clone()),
                }
            }
        }
        if !missing.is_empty() {
            let pairs: Vec<(String, String)> = {
                let set: BTreeSet<(String, String)> =
                    missing.iter().map(|k| (k.0.clone(), k.1.clone())).collect();
                set.into_iter().collect()
            };
            let diffs = diff_tree(repo, &pairs)?;
            let mut c = self.cache.lock().expect("git cache");
            for k in missing {
                let differ = diffs.get(&(k.0.clone(), k.1.clone()));
                let same =
                    files_by_tip.get(&k.0).is_none_or(|files| differ.is_none_or(|d| files.is_disjoint(d)));
                c.same.insert(key(&k), same);
                out.insert(k, same);
            }
            c.dirty = true;
        }
        Ok(out)
    }

    /// Diff de los commits propios de una rama (desde el padre del primero hasta el tip). `None` si la rama
    /// no tiene commits propios. Solo `git diff`.
    pub fn branch_diff(
        &self,
        r: &RepoConfig,
        tip: &str,
        rules: &Rules,
        max_chars: usize,
    ) -> Result<Option<BranchDiff>, GitError> {
        let path = Path::new(&r.path);
        let snap = repos::snapshot(path, rules)?;
        let env_shas: IndexMap<String, String> = r
            .envs
            .iter()
            .filter_map(|(env, rf)| snap.refs.get(rf).map(|sha| (env.clone(), sha.clone())))
            .collect();
        let graph = self.graph(path);
        let mut graph = graph.lock().expect("graph");
        graph.update(env_shas.values().map(String::as_str).chain([tip]))?;
        let own_all = graph.own_commits(tip, &env_shas);
        let Some((first, _)) = own_all.first() else { return Ok(None) };
        let parent = graph.parent(first);
        let base = if parent.is_empty() { first.clone() } else { parent };
        drop(graph);
        let stat = git(path, &["diff", "--stat", &base, tip], None, true)?.text();
        let diff = git(path, &["diff", "--unified=5", &base, tip], None, true)?.text();
        let cut = diff.chars().count() > max_chars;
        let diff = if cut { diff.chars().take(max_chars).collect() } else { diff };
        Ok(Some(BranchDiff {
            base,
            tip: tip.to_string(),
            commits: own_all.len(),
            stat: stat.trim().to_string(),
            diff,
            cut,
        }))
    }
}

fn trim<V>(map: &mut IndexMap<String, V>) {
    if map.len() > CACHE_DISK_MAX {
        let extra = map.len() - CACHE_DISK_MAX;
        map.drain(..extra);
    }
}

struct RepoOverview {
    info: RepoResult,
    entries: Vec<(BTreeSet<String>, BranchInfo)>,
    mentions: IndexMap<String, IndexMap<String, Mention>>,
}

pub(crate) fn cp_hit(sha: &str, picked: &HashSet<String>) -> bool {
    picked.contains(sha) || picked.iter().any(|p| p.len() < 40 && sha.starts_with(p.as_str()))
}

/// `{(a, b): archivos distintos entre a y b}` con `git diff-tree --stdin`: un proceso para muchos pares.
/// diff-tree encabeza cada resultado con el primer sha del par (y no imprime nada si no hay diferencias),
/// así que en cada llamada el primer sha no se repite.
pub(crate) fn diff_tree(
    repo: &Path,
    pairs: &[(String, String)],
) -> Result<HashMap<(String, String), BTreeSet<String>>, GitError> {
    let mut out: HashMap<(String, String), BTreeSet<String>> = HashMap::new();
    let mut pending: Vec<(String, String)> = vec![];
    for p in pairs {
        if !pending.contains(p) {
            pending.push(p.clone());
        }
    }
    while !pending.is_empty() {
        let (mut batch, mut rest) = (vec![], vec![]);
        let mut firsts = HashSet::new();
        for (a, b) in pending {
            if firsts.contains(&a) {
                rest.push((a, b));
            } else {
                firsts.insert(a.clone());
                batch.push((a, b));
            }
        }
        let other: HashMap<&str, &str> = batch.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let stdin: String = batch.iter().map(|(a, b)| format!("{a} {b}\n")).collect();
        let res =
            git(repo, &["diff-tree", "--stdin", "--name-only", "-r", "-z"], Some(stdin.as_bytes()), true)?;
        let text = String::from_utf8_lossy(&res.stdout);
        let mut cur: Option<(String, String)> = None;
        for tok in text.split('\0') {
            if let Some(b) = other.get(tok)
                && !out.contains_key(&(tok.to_string(), b.to_string()))
            {
                let k = (tok.to_string(), b.to_string());
                out.insert(k.clone(), BTreeSet::new());
                cur = Some(k);
            } else if !tok.is_empty()
                && let Some(k) = &cur
            {
                out.get_mut(k).expect("current pair").insert(tok.to_string());
            }
        }
        for p in batch {
            out.entry(p).or_default();
        }
        pending = rest;
    }
    Ok(out)
}
