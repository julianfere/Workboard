//! Comandos del panel Git (solo lectura): repos y entornos de la carpeta abierta, la vista general
//! (ramas de cada item y su estado por entorno), la firma para vigilar `.git`, los datos de la promoción y
//! el diff de los commits propios de un item. Repos: los configurados para esa carpeta en
//! `settings.json` o, si no hay, los repos hermanos detectados.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use indexmap::IndexMap;
use serde_json::{Value as Json, json};
use wb_core::Workflow;
use wb_git::{GitError, GitPanel, RepoConfig, Rules};

use crate::{Api, ApiError, ApiResult, Args, to_json};

/// Tope del diff de una rama para la revisión.
const DIFF_MAX_CHARS: usize = 400_000;

impl From<GitError> for ApiError {
    fn from(e: GitError) -> Self {
        let params: Vec<(&str, &str)> = e.params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        ApiError::new(e.code, &params)
    }
}

/// Estado del panel mientras la app está abierta.
#[derive(Default)]
pub(crate) struct GitState {
    panel: OnceLock<GitPanel>,
    /// Repos detectados por carpeta de trabajos (detectar corre git: una vez por carpeta).
    detected: Mutex<HashMap<String, Vec<RepoConfig>>>,
    /// Última vista general por carpeta.
    last: Mutex<HashMap<String, Json>>,
    /// Un solo cálculo a la vez: si hay uno en curso, el siguiente espera y reusa el resultado.
    busy: Mutex<()>,
}

fn rules(wf: &Workflow) -> ApiResult<Rules> {
    Rules::new(&wf.git.id_pattern, &wf.git.ignore_pattern, &wf.git.promote_prefix)
        .map_err(|e| ApiError::new("error.invalidWorkflow", &[("message", &e.to_string())]))
}

fn str_list(v: &Json) -> Vec<String> {
    v.as_array().map_or(vec![], |l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect())
}

/// Los entornos de un repo en el orden del flujo (la promoción a uno sale del anterior), sin vacíos ni
/// ambientes que el flujo no tiene.
fn ordered_envs(wf: &Workflow, envs: &IndexMap<String, String>) -> IndexMap<String, String> {
    wf.environments
        .iter()
        .filter_map(|e| {
            envs.get(&e.id).map(|r| r.trim()).filter(|r| !r.is_empty()).map(|r| (e.id.clone(), r.to_string()))
        })
        .collect()
}

impl Api {
    fn git_panel(&self) -> &GitPanel {
        self.git.panel.get_or_init(|| GitPanel::new(Some(self.config_dir.join("git_cache.json"))))
    }

    fn git_root(&self) -> ApiResult<String> {
        self.settings().root.ok_or_else(|| ApiError::new("error.noRoot", &[]))
    }

    /// Ramas candidatas de cada ambiente para detectar repos: las de los ajustes o las del flujo.
    fn env_refs(&self, wf: &Workflow) -> IndexMap<String, Vec<String>> {
        let own = self.settings().git.env_refs;
        let refs = own.as_ref().unwrap_or(&wf.git.env_refs);
        wf.environments.iter().filter_map(|e| refs.get(&e.id).map(|v| (e.id.clone(), v.clone()))).collect()
    }

    fn detected_repos(&self, wf: &Workflow, root: &str, fresh: bool) -> Vec<RepoConfig> {
        let mut cache = self.git.detected.lock().expect("detected");
        if fresh || !cache.contains_key(root) {
            let found = wb_git::detect_repos(Path::new(root), &self.env_refs(wf));
            cache.insert(root.to_string(), found);
        }
        cache[root].clone()
    }

    /// Repos que se revisan en la carpeta abierta.
    fn git_repos(&self, wf: &Workflow) -> ApiResult<Vec<RepoConfig>> {
        let root = self.git_root()?;
        let repos = match self.settings().git.repos.get(&root) {
            Some(own) => own.clone(),
            None => self.detected_repos(wf, &root, false),
        };
        Ok(repos.into_iter().map(|r| RepoConfig { envs: ordered_envs(wf, &r.envs), ..r }).collect())
    }

    fn forget_git(&self, root: &str) {
        self.git.last.lock().expect("last").remove(root);
        self.git.detected.lock().expect("detected").remove(root);
    }

    pub(crate) fn git_command(&self, cmd: &str, a: &Args) -> Option<ApiResult<Json>> {
        let out = match cmd {
            "git_config" => self.git_config(),
            "set_git_config" => self.set_git_config(a),
            "git_signature" => {
                self.workflow().and_then(|wf| Ok(json!(wb_git::watch_signature(&self.git_repos(&wf)?))))
            }
            "git_overview" => self.git_overview(a.bool("refresh")),
            "git_promotion" => self.git_promotion(a),
            "git_branch_diff" => self.git_branch_diff(a.str("name").unwrap_or_default(), false),
            "review_item" => self.review_item(a),
            _ => return None,
        };
        Some(out)
    }

    fn git_config(&self) -> ApiResult<Json> {
        let wf = self.workflow()?;
        let root = self.git_root()?;
        let custom = self.settings().git.repos.contains_key(&root);
        Ok(json!({
            "repos": self.git_repos(&wf)?,
            "detected": self.detected_repos(&wf, &root, true),
            "custom": custom,
            "envRefs": self.env_refs(&wf),
            "customEnvRefs": self.settings().git.env_refs.is_some(),
        }))
    }

    /// `repos`: la lista para la carpeta abierta (`null` = volver a los detectados). `envRefs`: ramas
    /// candidatas de cada ambiente (`null` = las del flujo; sin la clave = no cambia).
    fn set_git_config(&self, a: &Args) -> ApiResult<Json> {
        let wf = self.workflow()?;
        let root = self.git_root()?;
        let mut s = self.settings();
        match a.get("repos") {
            Json::Null => {
                s.git.repos.remove(&root);
            }
            list => {
                let raw: Vec<RepoConfig> = serde_json::from_value(list.clone())
                    .map_err(|_| ApiError::new("error.badArgument", &[("name", "repos")]))?;
                let mut clean: Vec<RepoConfig> = vec![];
                for r in raw {
                    let path = r.path.trim();
                    let path = dunce::canonicalize(path).unwrap_or_else(|_| Path::new(path).to_path_buf());
                    if !wb_git::is_repo(&path) {
                        return Err(ApiError::new("git.notRepo", &[("path", &path.display().to_string())]));
                    }
                    let path = path.display().to_string();
                    if clean.iter().any(|c| c.path.eq_ignore_ascii_case(&path)) {
                        continue;
                    }
                    let name = match r.name.trim() {
                        "" => Path::new(&path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        n => n.to_string(),
                    };
                    clean.push(RepoConfig {
                        name,
                        path,
                        envs: ordered_envs(&wf, &r.envs),
                        compare: r.compare,
                    });
                }
                s.git.repos.insert(root.clone(), clean);
            }
        }
        if let Some(refs) = a.0.get("envRefs") {
            s.git.env_refs = match refs {
                Json::Null => None,
                v => {
                    let raw: std::collections::BTreeMap<String, Vec<String>> =
                        serde_json::from_value(v.clone())
                            .map_err(|_| ApiError::new("error.badArgument", &[("name", "envRefs")]))?;
                    Some(
                        raw.into_iter()
                            .filter(|(k, _)| wf.environments.iter().any(|e| &e.id == k))
                            .map(|(k, v)| {
                                (
                                    k,
                                    v.into_iter()
                                        .map(|x| x.trim().to_string())
                                        .filter(|x| !x.is_empty())
                                        .collect(),
                                )
                            })
                            .collect(),
                    )
                }
            };
        }
        self.save_settings(s)?;
        self.forget_git(&root);
        self.git_config()
    }

    /// Vista general de la carpeta abierta (la última calculada, salvo `refresh`).
    fn git_overview(&self, refresh: bool) -> ApiResult<Json> {
        let root = self.git_root()?;
        if !refresh && let Some(hit) = self.git.last.lock().expect("last").get(&root) {
            return Ok(hit.clone());
        }
        let _busy = self.git.busy.lock().expect("git busy");
        if !refresh && let Some(hit) = self.git.last.lock().expect("last").get(&root) {
            return Ok(hit.clone());
        }
        let wf = self.workflow()?;
        let ws = self.workspace()?;
        let ids: BTreeSet<String> = ws.list_items()?.into_iter().flat_map(|it| it.ids).collect();
        let repos = self.git_repos(&wf)?;
        let ov = self.git_panel().overview(&repos, &ids, &rules(&wf)?);
        let mut out = to_json(ov)?;
        // Si cada repo se compara con los otros (alerta de repos desparejos).
        let compare: HashMap<&str, bool> = repos.iter().map(|r| (r.name.as_str(), r.compare)).collect();
        for r in out["repos"].as_array_mut().into_iter().flatten() {
            let c = r["name"].as_str().and_then(|n| compare.get(n)).copied().unwrap_or(true);
            r["compare"] = json!(c);
        }
        self.git.last.lock().expect("last").insert(root, out.clone());
        Ok(out)
    }

    fn git_promotion(&self, a: &Args) -> ApiResult<Json> {
        let wf = self.workflow()?;
        let ids: BTreeSet<String> = str_list(a.get("ids")).into_iter().collect();
        if ids.is_empty() {
            return Err(ApiError::new("promo.noIds", &[]));
        }
        let target = a.str("target")?;
        if !wf.environments.iter().any(|e| e.id == target) {
            return Err(ApiError::new("error.badArgument", &[("name", "target")]));
        }
        let repos = self.git_repos(&wf)?;
        to_json(self.git_panel().promotion(&repos, &ids, target, &rules(&wf)?))
    }

    /// Diff de los commits propios de cada rama de un item (para revisarlo con otro modelo). Con `fresh`, las
    /// ramas salen de una vista general recalculada (Revisar, como la v1) y no de la última.
    fn git_branch_diff(&self, name: &str, fresh: bool) -> ApiResult<Json> {
        let wf = self.workflow()?;
        let ws = self.workspace()?;
        let folder = ws.folder_of(name)?;
        let item = ws.build_item(&folder)?;
        let ov = self.git_overview(fresh)?;
        let repos = self.git_repos(&wf)?;
        let rules = rules(&wf)?;
        let mut seen = BTreeSet::new();
        let mut out = vec![];
        for id in &item.ids {
            for b in ov["byItem"][id].as_array().into_iter().flatten() {
                let (repo, tip) = (b["repo"].as_str().unwrap_or(""), b["tip"]["sha"].as_str().unwrap_or(""));
                if !seen.insert((repo.to_string(), tip.to_string())) {
                    continue;
                }
                let Some(r) = repos.iter().find(|r| r.name == repo) else { continue };
                if let Some(d) = self.git_panel().branch_diff(r, tip, &rules, DIFF_MAX_CHARS)? {
                    out.push(json!({"repo": repo, "path": r.path, "branch": b["name"], "diff": d}));
                }
            }
        }
        if out.is_empty() {
            return Err(ApiError::new("git.noOwnCommits", &[("item", name)]));
        }
        Ok(Json::Array(out))
    }

    /// Revisar: arma el paquete (diff de cada rama del item con git de solo lectura y el README con los
    /// criterios) en `<config>/review/<stamp>-<id>/` y abre una sesión en modo plan, con el modelo elegido, que
    /// deja el veredicto en una nota y los hallazgos bloqueantes como tareas.
    fn review_item(&self, a: &Args) -> ApiResult<Json> {
        let name = a.str("name")?;
        let model = a.get("model").as_str().unwrap_or("").trim().to_string();
        if !model.is_empty() && !valid_model(&model) {
            return Err(ApiError::new("review.badModel", &[]));
        }
        let ws = self.workspace()?;
        let folder = ws.folder_of(name)?;
        let item = ws.build_item(&folder)?;
        let Some(id) = item.ids.first().cloned().filter(|_| item.kind != wb_core::workspace::ItemKind::Group)
        else {
            return Err(ApiError::new("review.noId", &[]));
        };
        let diffs = self.git_branch_diff(&item.name, true)?;
        let lang = wb_i18n::resolve_lang(self.settings().lang.as_deref().unwrap_or(wb_i18n::DEFAULT_LANG));
        let t = |k: &str, p: &[(&str, &str)]| wb_i18n::t(lang, k, p);
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let dir = self.config_dir.join("review").join(format!("{stamp}-{}", file_part(&id)));
        std::fs::create_dir_all(&dir).map_err(wb_core::error::CoreError::from)?;
        let readme = folder.join(wb_core::workspace::README).display().to_string();
        let mut packet = vec![t("agent.review.packetTitle", &[("item", &item.name)]), String::new()];
        packet.push(t("agent.review.packetReadme", &[("path", &readme)]));
        packet.push(String::new());
        let mut used = BTreeSet::new();
        for d in diffs.as_array().into_iter().flatten() {
            let repo = d["repo"].as_str().unwrap_or("");
            let mut file = file_part(repo);
            let mut n = 2;
            while !used.insert(file.clone()) {
                file = format!("{}-{n}", file_part(repo));
                n += 1;
            }
            let diff_path = dir.join(format!("{file}.diff"));
            let cut = d["diff"]["cut"].as_bool().unwrap_or(false);
            let text =
                diff_file(d["diff"]["diff"].as_str().unwrap_or(""), cut, &t("agent.review.diffCut", &[]));
            std::fs::write(&diff_path, text).map_err(wb_core::error::CoreError::from)?;
            let short = |k: &str| d["diff"][k].as_str().unwrap_or("").chars().take(10).collect::<String>();
            let commits = d["diff"]["commits"].as_u64().unwrap_or(0).to_string();
            packet.push(format!("## {repo} · {}", d["branch"].as_str().unwrap_or("")));
            packet.push(String::new());
            packet.push(t("agent.review.packetRepo", &[("path", d["path"].as_str().unwrap_or(""))]));
            let mut line = t(
                "agent.review.packetCommits",
                &[("n", &commits), ("range", &format!("{}..{}", short("base"), short("tip")))],
            );
            if cut {
                line.push_str(&t("agent.review.packetCut", &[]));
            }
            packet.push(line);
            packet.push(t("agent.review.packetDiff", &[("path", &diff_path.display().to_string())]));
            packet.push(String::new());
            packet.push(format!(
                "```
{}
```",
                d["diff"]["stat"].as_str().unwrap_or("").trim_end()
            ));
            packet.push(String::new());
        }
        let packet_path = dir.join("REVIEW.md");
        std::fs::write(
            &packet_path,
            packet.join(
                "
",
            ),
        )
        .map_err(wb_core::error::CoreError::from)?;
        let title = t("agent.review.noteTitle", &[("id", &id)]);
        let prompt = t(
            "agent.review.prompt",
            &[
                ("id", &id),
                ("packet", &packet_path.display().to_string()),
                ("item", &item.name),
                ("title", &title),
                ("addNote", wb_sessions::names::ADD_NOTE),
                ("addTask", wb_sessions::names::ADD_TASK),
            ],
        );
        let session_name = if model.is_empty() {
            t("review.sessionName", &[("id", &id)])
        } else {
            t("review.sessionNameModel", &[("id", &id), ("model", &model)])
        };
        let n = |k: &str, d: u16| a.get(k).as_u64().map_or(d, |v| v.clamp(1, 1000) as u16);
        let agent = self.settings().agents.default_agent().to_string();
        let sess = self.session_manager()?.create(
            &ws,
            std::slice::from_ref(&item.name),
            &session_name,
            "plan",
            &agent,
            false,
            Some(&model),
            Some(&prompt),
            (n("cols", 120), n("rows", 30)),
        )?;
        Ok(json!({"session": sess, "packet": packet_path.display().to_string()}))
    }
}

/// El ambiente más avanzado (de los que tienen estado en `git.envStates`) donde el trabajo de estos ids ya
/// está en todos los repos involucrados que lo tienen configurado, con esos repos. Involucrado = tiene una
/// rama del item o un commit que lo menciona; los repos con error o sin comparar no cuentan.
pub(crate) fn env_reach(wf: &Workflow, ov: &Json, ids: &[String]) -> Option<(String, Vec<String>)> {
    const OK: [&str; 4] = ["merged", "equivalent", "content", "mentioned"];
    let branches: Vec<&Json> =
        ids.iter().flat_map(|id| ov["byItem"][id].as_array().into_iter().flatten()).collect();
    let mentioned = |repo: &str, env: &str| ids.iter().any(|id| !ov["mentions"][id][repo][env].is_null());
    let involved: Vec<&str> = ov["repos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["error"].is_null() && r["compare"] != json!(false))
        .filter_map(|r| r["name"].as_str())
        .filter(|name| {
            branches.iter().any(|b| b["repo"] == *name)
                || ids.iter().any(|id| ov["mentions"][id][*name].as_object().is_some_and(|m| !m.is_empty()))
        })
        .collect();
    for env in wf.environments.iter().rev().filter(|e| wf.git.env_states.contains_key(&e.id)) {
        let with_env: Vec<&str> = involved
            .iter()
            .copied()
            .filter(|name| {
                ov["repos"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|r| r["name"] == *name && !r["envs"][&env.id].is_null())
            })
            .collect();
        let has = |name: &str| {
            branches.iter().any(|b| {
                b["repo"] == name && OK.contains(&b["envs"][&env.id]["state"].as_str().unwrap_or(""))
            }) || mentioned(name, &env.id)
        };
        if !with_env.is_empty() && with_env.iter().all(|n| has(n)) {
            return Some((env.id.clone(), with_env.iter().map(|s| s.to_string()).collect()));
        }
    }
    None
}

impl Api {
    /// `env_reach` de cada item con ids, con la última vista general (si no hay repos o falla, nada).
    pub(crate) fn git_reach(
        &self,
        items: &[wb_core::workspace::Item],
    ) -> HashMap<String, (String, Vec<String>)> {
        let Ok(wf) = self.workflow() else { return HashMap::new() };
        if wf.git.env_states.is_empty() {
            return HashMap::new();
        }
        let Ok(ov) = self.git_overview(false) else { return HashMap::new() };
        items
            .iter()
            .filter(|it| !it.ids.is_empty())
            .filter_map(|it| env_reach(&wf, &ov, &it.ids).map(|r| (it.name.clone(), r)))
            .collect()
    }
}

/// Nombre de modelo para `--model`: letras, números y `.-_[]`.
fn valid_model(m: &str) -> bool {
    (2..=60).contains(&m.chars().count())
        && m.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '[' | ']'))
}

/// Parte segura para un nombre de archivo.
/// El `.diff` del paquete de Revisar: si se recortó, termina con la marca (como la v1, para que el agente sepa
/// que falta el resto).
fn diff_file(diff: &str, cut: bool, marker: &str) -> String {
    if cut { format!("{diff}\n{marker}\n") } else { diff.to_string() }
}

fn file_part(s: &str) -> String {
    let out: String =
        s.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-') { c } else { '_' }).collect();
    if out.is_empty() { "repo".into() } else { out }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    use super::*;

    fn git(dir: &Path, args: &[&str], stdin: Option<&[u8]>) {
        use std::io::Write;
        let mut child = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        if let Some(b) = stdin {
            child.stdin.take().unwrap().write_all(b).unwrap();
        }
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success(), "git {args:?}");
    }

    fn copy(src: &Path, dest: &Path) {
        std::fs::create_dir_all(dest).unwrap();
        for e in std::fs::read_dir(src).unwrap().flatten() {
            if e.path().is_dir() {
                copy(&e.path(), &dest.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), dest.join(e.file_name())).unwrap();
            }
        }
    }

    /// `proyecto/_work` (copia del fixture) y `proyecto/app` (el repo del golden de git).
    fn project(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("wb-api-git-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/fixtures");
        let work = base.join("proyecto/_work");
        copy(&fixtures.join("work"), &work);
        let app = base.join("proyecto/app");
        std::fs::create_dir_all(&app).unwrap();
        git(&app, &["init", "-q"], None);
        git(&app, &["config", "core.autocrlf", "false"], None);
        git(&app, &["fast-import", "--quiet"], Some(&std::fs::read(fixtures.join("repos/app.fi")).unwrap()));
        git(&app, &["config", "remote.origin.url", "https://dev.azure.com/acme/p/_git/app"], None);
        git(&app, &["checkout", "-q", "Feature/111111-login"], None);
        (work, base)
    }

    #[test]
    fn config_overview_promotion_and_diff() {
        let (work, base) = project("main");
        let api = Api::new(base.join("cfg"));
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": work})).unwrap();

        let cfg = api.dispatch("git_config", &json!({})).unwrap();
        assert_eq!(cfg["custom"], false);
        assert_eq!(cfg["repos"][0]["name"], "app");
        assert_eq!(
            cfg["repos"][0]["envs"],
            json!({"dev": "origin/development", "qa": "origin/qa", "prod": "origin/main"})
        );

        let ov = api.dispatch("git_overview", &json!({"refresh": true})).unwrap();
        assert_eq!(ov["repos"][0]["compare"], true);
        assert_eq!(ov["byItem"]["111111"][0]["name"], "Feature/111111-login");
        assert_eq!(ov["byItem"]["222222"][0]["envs"]["qa"]["state"], "equivalent");
        let sig = api.dispatch("git_signature", &json!({})).unwrap();
        assert!(sig.as_str().is_some_and(|s| !s.is_empty()));

        let promo = api.dispatch("git_promotion", &json!({"ids": ["111111"], "target": "qa"})).unwrap();
        assert_eq!(promo["repos"][0]["targetRef"], "origin/qa");
        assert_eq!(promo["repos"][0]["forge"], "azure");
        assert_eq!(promo["repos"][0]["sourceEnv"], "dev");
        let err = api.dispatch("git_promotion", &json!({"ids": [], "target": "qa"})).unwrap_err();
        assert_eq!(err.code, "promo.noIds");

        let diffs = api.dispatch("git_branch_diff", &json!({"name": "wi111111-login-con-sso"})).unwrap();
        assert_eq!(diffs[0]["diff"]["commits"], 3);
        api.dispatch("create_item", &json!({"folder": "wi123456-nada", "title": "Nada"})).unwrap();
        let none = api.dispatch("git_branch_diff", &json!({"name": "wi123456-nada"})).unwrap_err();
        assert_eq!(none.code, "git.noOwnCommits");

        // Repos a mano: sin QA, y uno que no es repo.
        let app = base.join("proyecto/app").display().to_string();
        let bad =
            api.dispatch("set_git_config", &json!({"repos": [{"name": "x", "path": base.join("cfg")}]}));
        assert_eq!(bad.unwrap_err().code, "git.notRepo");
        let cfg = api
            .dispatch(
                "set_git_config",
                &json!({"repos": [{"name": "", "path": app, "envs": {"prod": "origin/main", "dev": " origin/development ", "qa": ""}, "compare": false}]}),
            )
            .unwrap();
        assert_eq!(cfg["custom"], true);
        assert_eq!(cfg["repos"][0]["name"], "app");
        assert_eq!(cfg["repos"][0]["envs"], json!({"dev": "origin/development", "prod": "origin/main"}));
        let ov = api.dispatch("git_overview", &json!({})).unwrap();
        assert_eq!(ov["repos"][0]["compare"], false);
        assert!(ov["byItem"]["111111"][0]["envs"].get("qa").is_none());
        // Volver a los detectados.
        let cfg = api.dispatch("set_git_config", &json!({"repos": null})).unwrap();
        assert_eq!(cfg["custom"], false);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn review_item_writes_the_packet_and_opens_a_plan_session() {
        let (work, base) = project("review");
        let exe = std::env::current_exe().unwrap();
        let api = Api::new(base.join("cfg"));
        api.set_session_bins(exe.clone(), exe);
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es", "lang": "es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": work})).unwrap();
        let (cmd, args) =
            if cfg!(windows) { ("cmd", "/c echo {session_id}") } else { ("sh", "-c \"echo {session_id}\"") };
        api.dispatch("set_agents", &json!({"default": "custom", "custom": {"command": cmd, "args": args}}))
            .unwrap();

        let bad = api.dispatch("review_item", &json!({"name": "wi111111-login-con-sso", "model": "x; rm"}));
        assert_eq!(bad.unwrap_err().code, "review.badModel");
        api.dispatch("create_item", &json!({"folder": "fix-sin-id", "title": "Sin id"})).unwrap();
        let no_id = api.dispatch("review_item", &json!({"name": "fix-sin-id"}));
        assert_eq!(no_id.unwrap_err().code, "review.noId");

        // Revisar recalcula la vista general (v1 `app.py:725`): una rama nueva desde la última ya entra.
        api.dispatch("git_overview", &json!({})).unwrap();
        let app = base.join("proyecto/app");
        git(&app, &["checkout", "-q", "-b", "Feature/111111-extra", "origin/development"], None);
        std::fs::write(app.join("extra.txt"), "extra\n").unwrap();
        git(&app, &["add", "extra.txt"], None);
        let who = ["-c", "user.name=Test", "-c", "user.email=test@example.com"];
        git(&app, &[&who[..], &["commit", "-q", "-m", "WI 111111 extra"]].concat(), None);

        let out = api
            .dispatch(
                "review_item",
                &json!({"name": "wi111111-login-con-sso", "model": "opus", "cols": 100, "rows": 30}),
            )
            .unwrap();
        let packet = PathBuf::from(out["packet"].as_str().unwrap());
        assert!(packet.starts_with(base.join("cfg").join("review")));
        let text = std::fs::read_to_string(&packet).unwrap();
        assert!(text.starts_with("# Revisión de wi111111-login-con-sso"), "{text}");
        assert!(text.contains("## app · Feature/111111-login"), "{text}");
        assert!(text.contains("3 commits propios"), "{text}");
        assert!(text.contains("## app · Feature/111111-extra"), "{text}");
        let diff = packet.parent().unwrap().join("app.diff");
        assert!(std::fs::read_to_string(diff).unwrap().starts_with("diff --git"));
        let sess = &out["session"];
        assert_eq!(sess["mode"], "plan");
        assert_eq!(sess["plan"], false);
        assert_eq!(sess["model"], "opus");
        assert_eq!(sess["name"], "Revisar 111111 · opus");
        assert_eq!(sess["items"], json!(["wi111111-login-con-sso"]));
        api.shutdown();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_cut_diff_ends_with_the_marker() {
        assert_eq!(diff_file("diff --git a b", false, "… (diff recortado)"), "diff --git a b");
        assert_eq!(
            diff_file("diff --git a b", true, "… (diff recortado)"),
            "diff --git a b\n… (diff recortado)\n"
        );
        assert_eq!(wb_i18n::t("es", "agent.review.diffCut", &[]), "… (diff recortado)");
    }

    #[test]
    fn env_reach_needs_every_involved_repo() {
        let wf = wb_core::Workflow::preset("classic-es").unwrap();
        let envs = json!({"dev": "origin/development", "qa": "origin/qa", "prod": "origin/main"});
        let ov = json!({
            "repos": [
                {"name": "back", "envs": envs, "compare": true},
                {"name": "front", "envs": envs, "compare": true},
                {"name": "db", "envs": envs, "compare": false},
                {"name": "other", "envs": envs, "compare": true},
            ],
            "byItem": {
                "1": [
                    {"repo": "back", "name": "f/1", "envs": {"qa": {"state": "merged"}, "prod": {"state": "merged"}}},
                    {"repo": "front", "name": "f/1", "envs": {"qa": {"state": "equivalent"}, "prod": {"state": "pending"}}},
                    {"repo": "db", "name": "f/1", "envs": {"qa": {"state": "pending"}, "prod": {"state": "pending"}}},
                ],
                "2": [{"repo": "back", "name": "f/2", "envs": {"qa": {"state": "pending"}, "prod": {"state": "pending"}}}],
            },
            "mentions": {"2": {"other": {"prod": {"sha": "abc"}}}},
        });
        // back en PROD, front solo en QA (db no se compara) → QA en los dos.
        assert_eq!(
            env_reach(&wf, &ov, &["1".into()]),
            Some(("qa".into(), vec!["back".into(), "front".into()]))
        );
        // back no lo tiene en ningún ambiente: nada.
        assert_eq!(env_reach(&wf, &ov, &["2".into()]), None);
        assert_eq!(env_reach(&wf, &ov, &["9".into()]), None);
    }
}
