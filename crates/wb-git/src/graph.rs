//! Grafo de commits de un repo en memoria (como GitLens o Git Graph): un solo `git log` de las puntas
//! que interesan (entornos y ramas de items) en vez de un comando por pregunta. Los commits son
//! inmutables: las vueltas siguientes piden solo lo nuevo, excluyendo las puntas ya leídas.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use indexmap::IndexMap;

use crate::cmd::git;
use crate::{GitError, Mention, Rules, rules};

/// Tope de la línea principal (first-parent) de cada entorno.
pub const FP_MAX: usize = 20_000;
/// Tope de commits propios por rama.
pub const OWN_MAX: usize = 1000;
/// Entradas de los cachés por sha (un conjunto de hasta `FP_MAX` commits cada una).
const CACHE_MAX: usize = 64;
const FMT: &str = "%H%x1f%P%x1f%ct%x1f%s%x1f%b%x1e";

pub(crate) type Set = Arc<HashSet<u32>>;

pub(crate) struct Node {
    pub sha: String,
    pub parents: Vec<u32>,
    pub ts: i64,
    pub subject: String,
    /// Originales que nombra el trailer de un `cherry-pick -x`.
    pub cps: Vec<String>,
    /// Leído del log (los padres de un repo superficial pueden no estarlo).
    pub known: bool,
}

pub(crate) struct RepoGraph {
    repo: PathBuf,
    pub nodes: Vec<Node>,
    index: HashMap<String, u32>,
    tips: BTreeSet<String>,
    anc: HashMap<u32, Set>,
    fp: HashMap<u32, Set>,
    gen_: HashMap<u32, u32>,
}

impl RepoGraph {
    pub fn new(repo: PathBuf) -> Self {
        Self {
            repo,
            nodes: vec![],
            index: HashMap::new(),
            tips: BTreeSet::new(),
            anc: HashMap::new(),
            fp: HashMap::new(),
            gen_: HashMap::new(),
        }
    }

    fn intern(&mut self, sha: &str) -> u32 {
        if let Some(&i) = self.index.get(sha) {
            return i;
        }
        let i = self.nodes.len() as u32;
        self.nodes.push(Node {
            sha: sha.to_string(),
            parents: vec![],
            ts: 0,
            subject: String::new(),
            cps: vec![],
            known: false,
        });
        self.index.insert(sha.to_string(), i);
        i
    }

    pub fn id(&self, sha: &str) -> Option<u32> {
        self.index.get(sha).copied()
    }

    fn known(&self, sha: &str) -> bool {
        self.id(sha).is_some_and(|i| self.nodes[i as usize].known)
    }

    pub fn node(&self, i: u32) -> &Node {
        &self.nodes[i as usize]
    }

    /// Lee lo que falta para llegar a estas puntas.
    pub fn update<'a>(&mut self, tips: impl IntoIterator<Item = &'a str>) -> Result<(), GitError> {
        let new: BTreeSet<String> =
            tips.into_iter().filter(|t| !t.is_empty() && !self.known(t)).map(String::from).collect();
        if new.is_empty() {
            return Ok(());
        }
        let mut stdin = String::new();
        for t in &new {
            stdin.push_str(t);
            stdin.push('\n');
        }
        for t in &self.tips {
            stdin.push('^');
            stdin.push_str(t);
            stdin.push('\n');
        }
        let format = format!("--format={FMT}");
        let out = git(&self.repo, &["log", "--stdin", &format], Some(stdin.as_bytes()), true)?.text();
        for rec in out.split('\x1e') {
            let parts: Vec<&str> = rec.trim_matches('\n').splitn(5, '\x1f').collect();
            if parts.len() < 5 || parts[0].is_empty() {
                continue;
            }
            let (sha, parents, ts, subject, body) = (parts[0], parts[1], parts[2], parts[3], parts[4]);
            // El trailer a veces queda en el asunto (mensaje de una sola línea).
            let msg = format!("{subject}\n{body}");
            let cps =
                if msg.contains("cherry picked from commit") { rules::cp_trailer(&msg) } else { vec![] };
            let parents: Vec<u32> = parents.split_whitespace().map(|p| self.intern(p)).collect();
            let i = self.intern(sha);
            self.nodes[i as usize] = Node {
                sha: sha.to_string(),
                parents,
                ts: ts.parse().unwrap_or(0),
                subject: subject.to_string(),
                cps,
                known: true,
            };
        }
        self.tips.extend(new);
        Ok(())
    }

    /// Primer padre de un commit ("" si no tiene o no se conoce).
    pub fn parent(&self, sha: &str) -> String {
        self.id(sha)
            .map(|i| self.node(i))
            .filter(|n| n.known)
            .and_then(|n| n.parents.first())
            .map_or_else(String::new, |&p| self.node(p).sha.clone())
    }

    pub fn is_merge(&self, i: u32) -> bool {
        let n = self.node(i);
        n.known && n.parents.len() > 1
    }

    fn walk(&self, from: u32) -> HashSet<u32> {
        let mut seen = HashSet::new();
        let mut todo = vec![from];
        while let Some(s) = todo.pop() {
            if seen.insert(s) {
                let n = self.node(s);
                if n.known {
                    todo.extend(&n.parents);
                }
            }
        }
        seen
    }

    /// Todo lo alcanzable desde la rama (sin caché: se usa una vez por rama).
    pub fn reachable(&self, i: u32) -> HashSet<u32> {
        self.walk(i)
    }

    /// Todo lo alcanzable desde un entorno (incluido). Queda cacheado.
    pub fn ancestors(&mut self, i: u32) -> Set {
        if let Some(hit) = self.anc.get(&i) {
            return hit.clone();
        }
        let set = Arc::new(self.walk(i));
        if self.anc.len() >= CACHE_MAX {
            self.anc.clear();
        }
        self.anc.insert(i, set.clone());
        set
    }

    fn first_parent(&self, i: u32, limit: usize) -> Vec<u32> {
        let mut out = vec![];
        let mut s = Some(i);
        while let Some(x) = s {
            if out.len() >= limit {
                break;
            }
            out.push(x);
            let n = self.node(x);
            s = if n.known { n.parents.first().copied() } else { None };
        }
        out
    }

    fn first_parent_set(&mut self, i: u32) -> Set {
        if let Some(hit) = self.fp.get(&i) {
            return hit.clone();
        }
        let set: Set = Arc::new(self.first_parent(i, FP_MAX).into_iter().collect());
        if self.fp.len() >= CACHE_MAX {
            self.fp.clear();
        }
        self.fp.insert(i, set.clone());
        set
    }

    /// Commits propios de una rama: su línea first-parent hasta tocar la línea principal de algún
    /// entorno. `[(sha, es_merge)]` del más viejo al más nuevo.
    pub fn own_commits(&mut self, tip: &str, env_shas: &IndexMap<String, String>) -> Vec<(String, bool)> {
        let Some(tip) = self.id(tip) else { return vec![] };
        let ids: Vec<u32> = env_shas.values().filter_map(|s| self.id(s)).collect();
        let stops: Vec<Set> = ids.into_iter().map(|i| self.first_parent_set(i)).collect();
        let mut out = vec![];
        for s in self.first_parent(tip, OWN_MAX + 1) {
            if stops.iter().any(|st| st.contains(&s)) {
                break;
            }
            out.push((self.node(s).sha.clone(), self.is_merge(s)));
        }
        out.truncate(OWN_MAX);
        out.reverse();
        out
    }

    /// Distancia más larga hasta una raíz (un hijo siempre tiene más que sus padres).
    fn generation(&mut self, i: u32) -> u32 {
        if let Some(&g) = self.gen_.get(&i) {
            return g;
        }
        let mut todo = vec![i];
        while let Some(&s) = todo.last() {
            if self.gen_.contains_key(&s) {
                todo.pop();
                continue;
            }
            let parents: Vec<u32> =
                self.node(s).parents.iter().copied().filter(|&p| self.node(p).known).collect();
            let missing: Vec<u32> = parents.iter().copied().filter(|p| !self.gen_.contains_key(p)).collect();
            if missing.is_empty() {
                let g = 1 + parents.iter().map(|p| self.gen_[p]).max().unwrap_or(0);
                self.gen_.insert(s, g);
                todo.pop();
            } else {
                todo.extend(missing);
            }
        }
        self.gen_[&i]
    }

    /// Commits del entorno desde `since`, del más nuevo al más viejo (mismo segundo: el hijo primero).
    fn recent(&mut self, env: u32, since: i64) -> Vec<u32> {
        let anc = self.ancestors(env);
        let mut out: Vec<u32> = anc
            .iter()
            .copied()
            .filter(|&s| {
                let n = self.node(s);
                n.known && n.ts >= since
            })
            .collect();
        let gens: HashMap<u32, u32> = out.iter().map(|&s| (s, self.generation(s))).collect();
        out.sort_by_key(|s| (std::cmp::Reverse(self.node(*s).ts), std::cmp::Reverse(gens[s])));
        out
    }

    /// `{id: commit}` del commit más reciente del entorno cuyo mensaje menciona el item.
    pub fn mentions(
        &mut self,
        env_sha: &str,
        wanted: &BTreeSet<String>,
        rules: &Rules,
        since: i64,
    ) -> IndexMap<String, Mention> {
        let mut out = IndexMap::new();
        let Some(env) = self.id(env_sha) else { return out };
        for s in self.recent(env, since) {
            let n = self.node(s);
            for id in rules.ids_in_message(&n.subject).intersection(wanted) {
                out.entry(id.clone()).or_insert_with(|| Mention {
                    sha: n.sha.chars().take(9).collect(),
                    ts: n.ts,
                    subject: n.subject.clone(),
                });
            }
        }
        out
    }

    /// Commits originales que ya llegaron al entorno por `cherry-pick -x`. Sigue las cadenas: si PROD
    /// tiene un cherry-pick de un commit de QA que a su vez era cherry-pick del original, cuenta el original.
    pub fn picked_into(
        &mut self,
        env_shas: &IndexMap<String, String>,
        env: &str,
        since: i64,
    ) -> HashSet<String> {
        let mut maps: IndexMap<&str, Vec<(String, Vec<String>)>> = IndexMap::new();
        for (e, sha) in env_shas {
            let Some(i) = self.id(sha) else { continue };
            let picks = self
                .recent(i, since)
                .into_iter()
                .map(|s| self.node(s))
                .filter(|n| !n.cps.is_empty())
                .map(|n| (n.sha.clone(), n.cps.clone()))
                .collect();
            maps.insert(e.as_str(), picks);
        }
        closure(maps.get(env).map(Vec::as_slice).unwrap_or_default(), maps.values().flatten())
    }
}

/// Lo que nombran `roots` siguiendo las cadenas de `links` (commit → originales).
pub(crate) fn closure<'a>(
    roots: &[(String, Vec<String>)],
    links: impl Iterator<Item = &'a (String, Vec<String>)>,
) -> HashSet<String> {
    let mut found: HashSet<String> = roots.iter().flat_map(|(_, o)| o.iter().cloned()).collect();
    let mut map: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (k, v) in links {
        map.entry(k.as_str()).or_default().extend(v.iter().map(String::as_str));
    }
    let mut todo: Vec<String> = found.iter().cloned().collect();
    while let Some(x) = todo.pop() {
        for o in map.get(x.as_str()).into_iter().flatten() {
            if found.insert(o.to_string()) {
                todo.push(o.to_string());
            }
        }
    }
    found
}
