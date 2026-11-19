//! Promoción: los commits propios de las ramas de unos items que faltan en el entorno destino, por repo,
//! más los commits del entorno anterior que mencionan el item. Solo lectura: devuelve datos para que la UI
//! arme los comandos (cherry-pick o merge) que corre el usuario.
//!
//! Va directo a git (es a pedido y de pocos items): `rev-list`, `cherry`, `log`, `diff` y `for-each-ref`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::cmd::git;
use crate::graph::{FP_MAX, OWN_MAX, closure};
use crate::repos::{self, Group};
use crate::{CpMap, Forge, GitError, GitPanel, Msg, RepoConfig, Rules, cp_hit, is_repo, remote_web_url};

/// Tope de otras ramas de items que se revisan para detectar commits compartidos.
const OTHER_MAX: usize = 60;
const CACHE_MAX: usize = 64;

/// Por qué un commit es dudoso: `otherItem`, `shared`, `likely` o `merge`, con su texto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flag {
    pub kind: String,
    #[serde(flatten)]
    pub msg: Msg,
}

/// Un commit a promover (`on` = tildado por defecto).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub sha: String,
    pub short: String,
    pub ts: i64,
    pub author: String,
    pub parents: Vec<String>,
    pub subject: String,
    /// Rama de origen (o la rama del entorno anterior, para los mencionados).
    pub branch: String,
    pub ids: Vec<String>,
    pub merge: bool,
    pub flags: Vec<Flag>,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoBranch {
    /// Nombre de la rama o, con `from_env`, la rama del entorno anterior.
    pub name: String,
    pub ids: Vec<String>,
    pub locations: Vec<String>,
    pub commits: Vec<Row>,
    pub notes: Vec<Msg>,
    pub warnings: Vec<Msg>,
    pub already_cp: usize,
    /// Bloque de commits del entorno anterior que mencionan el item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoRepo {
    pub name: String,
    pub path: String,
    pub target_ref: String,
    pub remote: String,
    pub target_branch: String,
    pub web_url: Option<String>,
    pub forge: Option<Forge>,
    pub last_fetch: Option<f64>,
    pub branches: Vec<PromoBranch>,
    /// En el orden en que se aplican: cada rama en bloque, nunca intercaladas.
    pub commits: Vec<Row>,
    pub source_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PromoRepoResult {
    Ok(Box<PromoRepo>),
    Err { name: String, error: Msg },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Promotion {
    pub target: String,
    pub ids: Vec<String>,
    pub repos: Vec<PromoRepoResult>,
}

#[derive(Debug, Clone)]
struct CommitInfo {
    sha: String,
    short: String,
    ts: i64,
    author: String,
    parents: Vec<String>,
    subject: String,
}

impl CommitInfo {
    fn row(&self, branch: &str, ids: Vec<String>, merge: bool, flags: Vec<Flag>, on: bool) -> Row {
        Row {
            sha: self.sha.clone(),
            short: self.short.clone(),
            ts: self.ts,
            author: self.author.clone(),
            parents: self.parents.clone(),
            subject: self.subject.clone(),
            branch: branch.to_string(),
            ids,
            merge,
            flags,
            on,
        }
    }
}

fn flag(kind: &str, code: &str, params: &[(&str, &str)]) -> Flag {
    Flag { kind: kind.into(), msg: Msg::new(code, params) }
}

fn lines_plus(text: &str) -> HashSet<String> {
    text.lines().filter_map(|l| l.strip_prefix("+ ")).map(|s| s.trim().to_string()).collect()
}

/// Lo que comparten los cálculos de un repo.
struct Ctx<'a> {
    path: &'a Path,
    wanted: &'a BTreeSet<String>,
    rules: &'a Rules,
    target: &'a str,
    target_sha: String,
    since: i64,
    /// (autor, asunto) de los commits recientes del destino (se lee solo si hace falta).
    subjects: Option<HashSet<(String, String)>>,
}

impl GitPanel {
    /// Datos para promover los items `ids` al entorno `target` en cada repo que lo tiene.
    pub fn promotion(
        &self,
        repos: &[RepoConfig],
        ids: &BTreeSet<String>,
        target: &str,
        rules: &Rules,
    ) -> Promotion {
        let mut out = vec![];
        for r in repos.iter().filter(|r| r.envs.contains_key(target)) {
            match self.promotion_repo(r, ids, target, rules) {
                Ok(x) => out.push(x),
                Err(e) => out.push(PromoRepoResult::Err { name: r.name.clone(), error: e.msg() }),
            }
        }
        Promotion { target: target.to_string(), ids: ids.iter().cloned().collect(), repos: out }
    }

    fn first_parent_set_git(&self, path: &Path, sha: &str) -> Result<Arc<HashSet<String>>, GitError> {
        let key = (path.display().to_string(), sha.to_string());
        if let Some(hit) = self.fp.lock().expect("fp").get(&key) {
            return Ok(hit.clone());
        }
        let max = format!("--max-count={FP_MAX}");
        let out = git(path, &["rev-list", "--first-parent", &max, sha], None, true)?.text();
        let set: Arc<HashSet<String>> = Arc::new(out.split_whitespace().map(String::from).collect());
        let mut fp = self.fp.lock().expect("fp");
        if fp.len() >= CACHE_MAX {
            fp.clear();
        }
        fp.insert(key, set.clone());
        Ok(set)
    }

    /// Commits propios de una rama (como en el panel, pero con git). `found` = se encontró dónde nace.
    fn own_commits_git(
        &self,
        path: &Path,
        tip: &str,
        env_shas: &IndexMap<String, String>,
    ) -> Result<(Vec<(String, bool)>, bool), GitError> {
        let stops: Vec<Arc<HashSet<String>>> =
            env_shas.values().map(|s| self.first_parent_set_git(path, s)).collect::<Result<_, _>>()?;
        let max = format!("--max-count={}", OWN_MAX + 1);
        let out = git(path, &["rev-list", "--first-parent", &max, "--parents", tip], None, true)?.text();
        let (mut list, mut found) = (vec![], false);
        for line in out.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let Some(first) = parts.first() else { continue };
            if stops.iter().any(|s| s.contains(*first)) {
                found = true;
                break;
            }
            list.push((first.to_string(), parts.len() > 2));
        }
        list.truncate(OWN_MAX);
        list.reverse();
        Ok((list, found))
    }

    /// `{commit: originales}` de los cherry-pick -x de un entorno (trailer), de la ventana de días.
    fn cp_map(&self, path: &Path, sha: &str, since: i64) -> Result<CpMap, GitError> {
        let key = (path.display().to_string(), sha.to_string());
        if let Some(hit) = self.cp.lock().expect("cp").get(&key) {
            return Ok(hit.clone());
        }
        let since_arg = format!("--since=@{since}");
        let log = git(
            path,
            &["log", sha, &since_arg, "--grep=cherry picked from commit", "--format=%H%x00%B%x1e"],
            None,
            false,
        )?
        .text();
        let mut out = vec![];
        for rec in log.split('\x1e') {
            let (h, body) = rec.trim().split_once('\0').unwrap_or((rec.trim(), ""));
            let orig = crate::cp_trailer(body);
            if !h.is_empty() && !orig.is_empty() {
                out.push((h.to_string(), orig));
            }
        }
        let out = Arc::new(out);
        let mut cp = self.cp.lock().expect("cp");
        if cp.len() >= CACHE_MAX {
            cp.clear();
        }
        cp.insert(key, out.clone());
        Ok(out)
    }

    fn cp_links(
        &self,
        path: &Path,
        env_shas: &IndexMap<String, String>,
        since: i64,
    ) -> Result<IndexMap<String, CpMap>, GitError> {
        env_shas.iter().map(|(e, sha)| Ok((e.clone(), self.cp_map(path, sha, since)?))).collect()
    }

    fn promotion_repo(
        &self,
        r: &RepoConfig,
        wanted: &BTreeSet<String>,
        target: &str,
        rules: &Rules,
    ) -> Result<PromoRepoResult, GitError> {
        let path = Path::new(&r.path);
        if !is_repo(path) {
            return Err(GitError::new("git.notRepo", &[("path", &r.path)]));
        }
        let snap = repos::snapshot(path, rules)?;
        let env_shas: IndexMap<String, String> = r
            .envs
            .iter()
            .filter_map(|(env, rf)| snap.refs.get(rf).map(|sha| (env.clone(), sha.clone())))
            .collect();
        let target_ref = r.envs[target].clone();
        let Some(target_sha) = env_shas.get(target).cloned() else {
            return Ok(PromoRepoResult::Err {
                name: r.name.clone(),
                error: Msg::new("git.missingRef", &[("ref", &target_ref)]),
            });
        };
        let (remote, target_branch) = target_ref.split_once('/').unwrap_or((&target_ref, ""));
        let mut cx = Ctx { path, wanted, rules, target, target_sha, since: self.since(), subjects: None };
        let links = self.cp_links(path, &env_shas, cx.since)?;
        let picked_into = closure(
            links.get(target).map(|v| v.as_slice()).unwrap_or_default(),
            links.values().flat_map(|v| v.iter()),
        );
        let mut fp_sets: IndexMap<&str, Arc<HashSet<String>>> = IndexMap::new();
        for (env, sha) in &env_shas {
            fp_sets.insert(env.as_str(), self.first_parent_set_git(path, sha)?);
        }
        let mut branches: Vec<PromoBranch> = vec![];
        let mut picks: Vec<Vec<String>> = vec![];
        let mut rows: Vec<Row> = vec![];

        // 1) Ramas con el id del item en el nombre.
        for g in &snap.groups {
            let hits: Vec<String> = g.nums.intersection(wanted).cloned().collect();
            if hits.is_empty() {
                continue;
            }
            let tip = g.tip.sha.as_str();
            let mut locations: Vec<String> = g.locations.keys().cloned().collect();
            locations.sort();
            let mut entry = PromoBranch {
                name: g.name.clone(),
                ids: g.nums.iter().cloned().collect(),
                locations,
                commits: vec![],
                notes: vec![],
                warnings: vec![],
                already_cp: 0,
                from_env: None,
            };
            let mut mine: Vec<String> = vec![];
            let (own_all, found) = self.own_commits_git(path, tip, &env_shas)?;
            if !found {
                // Sin el punto de partida la lista traería historia ajena: no se propone nada.
                entry.warnings.push(Msg::new("promo.warn.noBase", &[]));
                branches.push(entry);
                picks.push(mine);
                continue;
            }
            let local = g.locations.get("local");
            let remote_sha = g.locations.iter().find(|(k, _)| *k != "local").map(|(_, v)| v);
            if let (Some(l), Some(rs)) = (local, remote_sha)
                && l != rs
                && g.tip.loc == "local"
            {
                entry.warnings.push(Msg::new("promo.warn.unpushed", &[]));
            }
            if own_all.is_empty() {
                entry.notes.push(Msg::new("promo.note.fastForward", &[]));
                branches.push(entry);
                picks.push(mine);
                continue;
            }
            let own: Vec<String> = own_all.iter().filter(|(_, m)| !m).map(|(s, _)| s.clone()).collect();
            let missing = self.missing_in(&cx, tip, &own_all[0].0)?;
            let not_target = format!("^{}", cx.target_sha);
            let reach: HashSet<String> = git(path, &["rev-list", tip, &not_target], None, true)?
                .text()
                .split_whitespace()
                .map(String::from)
                .collect();
            let shared = self.shared_with_others(path, tip, &own, &snap.groups, wanted, &env_shas)?;
            let info = commit_info(path, own_all.iter().map(|(s, _)| s.as_str()))?;
            let mut env_merges = 0;
            let taken: HashSet<String> = rows.iter().map(|x| x.sha.clone()).collect();
            for (sha, is_merge) in &own_all {
                // Ya alcanzable desde el destino, o ya lo propuso otra rama del mismo item.
                if !reach.contains(sha) || taken.contains(sha) {
                    continue;
                }
                let Some(c) = info.get(sha) else { continue };
                if *is_merge {
                    let src = fp_sets
                        .iter()
                        .find(|(_, fp)| c.parents.len() > 1 && fp.contains(&c.parents[1]))
                        .map(|(e, _)| *e);
                    let f = match src {
                        Some(src) => {
                            env_merges += 1;
                            flag("merge", "promo.flag.mergeFromEnv", &[("env", src), ("target", target)])
                        }
                        None => flag("merge", "promo.flag.mergeOther", &[]),
                    };
                    rows.push(c.row(&g.name, hits.clone(), true, vec![f], false));
                    mine.push(sha.clone());
                    continue;
                }
                if !missing.contains(sha) {
                    continue; // mismo patch ya en el destino
                }
                if cp_hit(sha, &picked_into) {
                    entry.already_cp += 1;
                    continue;
                }
                let (flags, on) = self.flags_for(&mut cx, sha, c, &shared)?;
                rows.push(c.row(&g.name, hits.clone(), false, flags, on));
                mine.push(sha.clone());
            }
            let n = |v: usize| v.to_string();
            if env_merges > 0 {
                entry
                    .warnings
                    .push(Msg::new("promo.warn.envMerges", &[("n", &n(env_merges)), ("target", target)]));
            }
            if entry.already_cp > 0 {
                entry.notes.push(Msg::new(
                    "promo.note.alreadyCp",
                    &[("n", &n(entry.already_cp)), ("target", target)],
                ));
            }
            let pending: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter(|(_, x)| mine.contains(&x.sha) && !x.merge)
                .map(|(i, _)| i)
                .collect();
            if !pending.is_empty() && self.files_in_target(&cx, &own_all, tip)? {
                entry.warnings.push(Msg::new("promo.warn.sameContent", &[("target", target)]));
                for i in pending {
                    rows[i].on = false;
                    rows[i].flags.push(flag("likely", "promo.flag.sameContent", &[("target", target)]));
                }
            }
            if mine.is_empty() {
                entry.notes.push(Msg::new("promo.note.nothing", &[("target", target)]));
            }
            branches.push(entry);
            picks.push(mine);
        }

        // 2) Commits del entorno anterior que mencionan el item, aunque no haya rama con su id. Cada
        //    entorno sale del anterior en el orden configurado.
        let order: Vec<&String> = env_shas.keys().collect();
        let source_env =
            order.iter().position(|e| *e == target).filter(|&i| i > 0).map(|i| order[i - 1].clone());
        if let Some(source) = &source_env {
            let source_sha = &env_shas[source];
            let mentioned = self.mentioned_commits(&cx, source_sha)?;
            if !mentioned.is_empty() {
                let cherry = git(path, &["cherry", &cx.target_sha, source_sha], None, false)?.text();
                let missing = lines_plus(&cherry);
                let taken: HashSet<String> = rows.iter().map(|x| x.sha.clone()).collect();
                let taken_msgs: HashSet<(String, String)> =
                    rows.iter().map(|x| (x.author.clone(), x.subject.clone())).collect();
                let all_links: Vec<(String, Vec<String>)> =
                    links.values().flat_map(|v| v.iter().cloned()).collect();
                let extra: Vec<&(String, BTreeSet<String>)> = mentioned
                    .iter()
                    .filter(|(sha, _)| {
                        missing.contains(sha) && !taken.contains(sha) && !cp_hit(sha, &picked_into)
                    })
                    .filter(|(sha, _)| {
                        let roots: Vec<(String, Vec<String>)> =
                            all_links.iter().filter(|(k, _)| k == sha).cloned().collect();
                        let origins = closure(&roots, all_links.iter());
                        !origins.iter().any(|o| taken.iter().any(|t| t.starts_with(o.as_str())))
                    })
                    .collect();
                let block = r.envs[source].clone();
                let ids: BTreeSet<String> = mentioned.iter().flat_map(|(_, w)| w.iter().cloned()).collect();
                let mut entry = PromoBranch {
                    name: block.clone(),
                    ids: ids.into_iter().collect(),
                    locations: vec![],
                    commits: vec![],
                    notes: vec![],
                    warnings: vec![],
                    already_cp: 0,
                    from_env: Some(source.clone()),
                };
                if extra.is_empty() {
                    entry
                        .notes
                        .push(Msg::new("promo.note.mentionsDone", &[("source", source), ("target", target)]));
                }
                let info = commit_info(path, extra.iter().map(|(s, _)| s.as_str()))?;
                let mut mine = vec![];
                for (sha, w) in extra {
                    let Some(c) = info.get(sha) else { continue };
                    // Copia (cherry-pick sin -x) de un commit que ya se propone desde la rama.
                    if taken_msgs.contains(&(c.author.clone(), c.subject.clone())) {
                        continue;
                    }
                    // Llegó porque el merge que lo trajo menciona el item, pero el commit es de otro.
                    let nums = rules.ids_in_message(&c.subject);
                    if !nums.is_empty() && nums.is_disjoint(wanted) {
                        continue;
                    }
                    let (flags, on) = self.flags_for(&mut cx, sha, c, &HashMap::new())?;
                    rows.push(c.row(&block, w.iter().cloned().collect(), false, flags, on));
                    mine.push(sha.clone());
                }
                branches.push(entry);
                picks.push(mine);
            }
        }

        // Orden de aplicación: cada rama en bloque (en su orden first-parent), las ramas por su commit más
        // viejo y al final lo que sale del entorno anterior.
        let by_sha: HashMap<String, Row> = rows.into_iter().map(|x| (x.sha.clone(), x)).collect();
        for (b, shas) in branches.iter_mut().zip(&picks) {
            b.commits = shas.iter().filter_map(|s| by_sha.get(s).cloned()).collect();
        }
        let mut own_blocks: Vec<&PromoBranch> =
            branches.iter().filter(|b| !b.commits.is_empty() && b.from_env.is_none()).collect();
        own_blocks.sort_by_key(|b| b.commits[0].ts);
        let commits: Vec<Row> = own_blocks
            .into_iter()
            .chain(branches.iter().filter(|b| b.from_env.is_some()))
            .flat_map(|b| b.commits.iter().cloned())
            .collect();
        let web = remote_web_url(path, remote);
        Ok(PromoRepoResult::Ok(Box::new(PromoRepo {
            name: r.name.clone(),
            path: r.path.clone(),
            target_ref: target_ref.clone(),
            remote: remote.to_string(),
            target_branch: target_branch.to_string(),
            web_url: web.as_ref().map(|(_, u)| u.clone()),
            forge: web.map(|(f, _)| f),
            last_fetch: snap.last_fetch,
            branches,
            commits,
            source_env,
        })))
    }

    /// Marcas de un commit y si va tildado.
    fn flags_for(
        &self,
        cx: &mut Ctx,
        sha: &str,
        c: &CommitInfo,
        shared: &HashMap<String, String>,
    ) -> Result<(Vec<Flag>, bool), GitError> {
        let (mut flags, mut on) = (vec![], true);
        let nums = cx.rules.ids_in_message(&c.subject);
        let mine = !nums.is_disjoint(cx.wanted);
        let other: Vec<&String> = nums.difference(cx.wanted).collect();
        if !other.is_empty() && !mine {
            let ids = other.iter().map(|o| format!("#{o}")).collect::<Vec<_>>().join(", ");
            flags.push(flag("otherItem", "promo.flag.otherItem", &[("ids", &ids)]));
            on = false;
        }
        if let Some(branch) = shared.get(sha) {
            flags.push(flag("shared", "promo.flag.shared", &[("branch", branch)]));
            on = on && mine;
        }
        if cx.subjects.is_none() {
            let since = format!("--since=@{}", cx.since);
            let log = git(
                cx.path,
                &["log", &cx.target_sha, &since, "--no-merges", "--format=%an%x09%s"],
                None,
                false,
            )?
            .text();
            cx.subjects = Some(
                log.lines()
                    .filter_map(|l| l.split_once('\t'))
                    .map(|(a, s)| (a.to_string(), s.to_string()))
                    .collect(),
            );
        }
        if cx.subjects.as_ref().is_some_and(|s| s.contains(&(c.author.clone(), c.subject.clone()))) {
            flags.push(flag("likely", "promo.flag.sameMessage", &[("target", cx.target)]));
            on = false;
        }
        Ok((flags, on))
    }

    fn parent_git(&self, path: &Path, sha: &str) -> Result<String, GitError> {
        let out = git(path, &["rev-list", "--parents", "-n", "1", sha], None, false)?;
        if !out.ok {
            return Ok(String::new());
        }
        Ok(out.text().split_whitespace().nth(1).unwrap_or("").to_string())
    }

    /// Commits entre `oldest` y `tip` que no tienen equivalente en el destino (`git cherry`, los `+`).
    fn missing_in(&self, cx: &Ctx, tip: &str, oldest: &str) -> Result<HashSet<String>, GitError> {
        let parent = self.parent_git(cx.path, oldest)?;
        let mut args = vec!["cherry", cx.target_sha.as_str(), tip];
        if !parent.is_empty() {
            args.push(&parent);
        }
        Ok(lines_plus(&git(cx.path, &args, None, false)?.text()))
    }

    /// Commits propios que también están en la línea de otra rama de item (rama apilada sobre otra
    /// feature): `{sha: nombre de la otra rama}`. Las ramas que se crearon encima de esta no cuentan.
    fn shared_with_others(
        &self,
        path: &Path,
        tip: &str,
        own: &[String],
        groups: &[Group],
        wanted: &BTreeSet<String>,
        env_shas: &IndexMap<String, String>,
    ) -> Result<HashMap<String, String>, GitError> {
        let mut out = HashMap::new();
        let Some(first) = own.first() else { return Ok(out) };
        let containing: HashSet<String> = git(
            path,
            &["for-each-ref", "--contains", first, "--format=%(objectname)", "refs/heads", "refs/remotes"],
            None,
            false,
        )?
        .text()
        .split_whitespace()
        .map(String::from)
        .collect();
        let own_set: HashSet<&String> = own.iter().collect();
        let mut checked = 0;
        for g in groups.iter().filter(|g| g.nums.is_disjoint(wanted)) {
            let tips: BTreeSet<&String> =
                g.locations.values().filter(|s| containing.contains(*s) && s.as_str() != tip).collect();
            for other_tip in tips {
                if checked >= OTHER_MAX {
                    return Ok(out);
                }
                checked += 1;
                let (line, _) = self.own_commits_git(path, other_tip, env_shas)?;
                // La otra rama está apilada sobre esta: los commits son de esta.
                if line.iter().any(|(s, _)| s == tip) {
                    continue;
                }
                for (sha, _) in line.iter().filter(|(s, _)| own_set.contains(s)) {
                    out.entry(sha.clone()).or_insert_with(|| g.name.clone());
                }
            }
        }
        Ok(out)
    }

    /// Commits del entorno origen cuyo mensaje menciona alguno de los items. Un merge de PR se expande a
    /// los commits que trajo (`M^1..M`), en orden topológico. `[(sha, ids)]` del más viejo al más nuevo.
    fn mentioned_commits(
        &self,
        cx: &Ctx,
        source_sha: &str,
    ) -> Result<Vec<(String, BTreeSet<String>)>, GitError> {
        let since = format!("--since=@{}", cx.since);
        let log = git(
            cx.path,
            &["log", source_sha, &since, "--topo-order", "--format=%H%x09%P%x09%s"],
            None,
            false,
        )?
        .text();
        let mut found: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut order: Vec<String> = vec![];
        for line in log.lines() {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() < 3 {
                continue;
            }
            let hits: BTreeSet<String> =
                cx.rules.ids_in_message(parts[2]).intersection(cx.wanted).cloned().collect();
            if hits.is_empty() {
                continue;
            }
            let parents: Vec<&str> = parts[1].split_whitespace().collect();
            let shas: Vec<String> = if parents.len() > 1 {
                let range = format!("{}..{}", parents[0], parts[0]);
                git(cx.path, &["rev-list", "--no-merges", "--topo-order", &range], None, true)?
                    .text()
                    .split_whitespace()
                    .map(String::from)
                    .collect()
            } else {
                vec![parts[0].to_string()]
            };
            for sha in shas {
                if !found.contains_key(&sha) {
                    order.push(sha.clone());
                }
                found.entry(sha).or_default().extend(hits.iter().cloned());
            }
        }
        Ok(order
            .into_iter()
            .rev()
            .map(|s| {
                let ids = found.remove(&s).unwrap_or_default();
                (s, ids)
            })
            .collect())
    }

    /// ¿Los archivos que tocó la rama están idénticos en el destino? (squash o merge manual)
    fn files_in_target(&self, cx: &Ctx, own_all: &[(String, bool)], tip: &str) -> Result<bool, GitError> {
        let first = &own_all[0].0;
        let parent = self.parent_git(cx.path, first)?;
        let from = if parent.is_empty() { first.as_str() } else { parent.as_str() };
        let names = |a: &str| -> Result<HashSet<String>, GitError> {
            Ok(git(cx.path, &["diff", "--name-only", a, tip], None, true)?
                .text()
                .split('\n')
                .filter(|f| !f.is_empty())
                .map(String::from)
                .collect())
        };
        let files = names(from)?;
        if files.is_empty() {
            return Ok(false);
        }
        let differ = names(&cx.target_sha)?;
        Ok(files.is_disjoint(&differ))
    }
}

/// Datos de cada commit (`git log --no-walk --stdin`).
fn commit_info<'a>(
    path: &Path,
    shas: impl Iterator<Item = &'a str>,
) -> Result<HashMap<String, CommitInfo>, GitError> {
    let stdin: String = shas.map(|s| format!("{s}\n")).collect();
    if stdin.is_empty() {
        return Ok(HashMap::new());
    }
    let out = git(
        path,
        &["log", "--no-walk=unsorted", "--stdin", "--format=%H%x09%h%x09%ct%x09%an%x09%P%x09%s"],
        Some(stdin.as_bytes()),
        true,
    )?
    .text();
    let mut map = HashMap::new();
    for line in out.lines() {
        let mut p: Vec<&str> = line.splitn(6, '\t').collect();
        p.resize(6, "");
        if p[0].is_empty() {
            continue;
        }
        map.insert(
            p[0].to_string(),
            CommitInfo {
                sha: p[0].to_string(),
                short: p[1].to_string(),
                ts: p[2].parse().unwrap_or(0),
                author: p[3].to_string(),
                parents: p[4].split_whitespace().map(String::from).collect(),
                subject: p[5].to_string(),
            },
        );
    }
    Ok(map)
}
