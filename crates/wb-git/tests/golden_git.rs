//! Golden de la v1 (`tests/golden/cases/git.json`) sobre el repo sintético de
//! `tests/golden/fixtures/repos/app.fi`, con las reglas del preset `classic-es`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use indexmap::IndexMap;
use serde_json::{Value, json};
use wb_git::{GitPanel, RepoConfig, Rules};

const NOW: i64 = 1_791_000_000;
const WIS: [&str; 8] = ["111111", "222222", "333333", "444444", "555555", "666666", "777777", "999999"];

fn repo_root() -> PathBuf {
    dunce::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

fn golden() -> Value {
    let p = repo_root().join("tests/golden/cases/git.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn rules() -> Rules {
    let wf = wb_core::Workflow::preset("classic-es").unwrap();
    Rules::new(&wf.git.id_pattern, &wf.git.ignore_pattern, &wf.git.promote_prefix).unwrap()
}

fn setup_git(dir: &Path, args: &[&str], stdin: Option<&[u8]>) {
    use std::io::Write;
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    if let Some(bytes) = stdin {
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success(), "git {args:?}");
}

/// Carpeta temporal con `proyecto/_work`, el repo `app` del fixture, un repo sin entornos y una carpeta
/// que no es repo (como `gen_git.py`).
struct Fixture {
    tmp: PathBuf,
    project: PathBuf,
    app: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let tmp = std::env::temp_dir().join(format!("wb-git-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let tmp = dunce::canonicalize(&tmp).unwrap();
        let project = tmp.join("proyecto");
        std::fs::create_dir_all(project.join("_work")).unwrap();
        let app = project.join("app");
        std::fs::create_dir_all(&app).unwrap();
        let fi = std::fs::read(repo_root().join("tests/golden/fixtures/repos/app.fi")).unwrap();
        setup_git(&app, &["init", "-q"], None);
        setup_git(&app, &["config", "core.autocrlf", "false"], None);
        setup_git(&app, &["fast-import", "--quiet"], Some(&fi));
        setup_git(&app, &["config", "remote.origin.url", "https://example.com/acme/app.git"], None);
        setup_git(&app, &["config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"], None);
        setup_git(&app, &["checkout", "-q", "Feature/111111-login"], None);
        std::fs::write(app.join("pendiente.txt"), "x\n").unwrap();
        std::fs::create_dir_all(project.join("vault")).unwrap();
        setup_git(&project.join("vault"), &["init", "-q"], None);
        std::fs::create_dir_all(project.join("docs")).unwrap();
        Self { tmp, project, app }
    }

    fn repo(&self, envs: &[(&str, &str)]) -> RepoConfig {
        RepoConfig {
            name: "app".into(),
            path: self.app.display().to_string(),
            envs: envs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            compare: true,
        }
    }

    /// La salida del port en la forma de la v1: textos en español, nombres de la v1, sin campos de reloj.
    fn v1(&self, v: impl serde::Serialize) -> Value {
        let mut v = to_v1(serde_json::to_value(v).unwrap());
        mention_blocks(&mut v);
        scrub(v, &self.tmp.display().to_string())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

const ENVS: [(&str, &str); 3] = [("DEV", "origin/development"), ("QA", "origin/qa"), ("PROD", "origin/main")];

fn es(msg: &Value) -> String {
    let params: Vec<(String, String)> = msg["params"]
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default();
    let p: Vec<(&str, &str)> = params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    wb_i18n::t("es", msg["code"].as_str().unwrap(), &p)
}

fn to_v1(v: Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.into_iter().map(to_v1).collect()),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                match k.as_str() {
                    "computedAt" | "seconds" | "lastFetch" | "forge" | "compare" => continue,
                    "ids" => {
                        out.insert("wis".into(), v);
                    }
                    "byItem" => {
                        out.insert("byWi".into(), to_v1(v));
                    }
                    "error" if v.is_object() => {
                        out.insert(k, Value::String(es(&v)));
                    }
                    "notes" | "warnings" => {
                        let texts = v.as_array().unwrap().iter().map(|m| Value::String(es(m))).collect();
                        out.insert(k, Value::Array(texts));
                    }
                    "flags" => {
                        let flags = v
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|f| {
                                let kind = if f["kind"] == "otherItem" {
                                    "otherwi"
                                } else {
                                    f["kind"].as_str().unwrap()
                                };
                                json!({"kind": kind, "text": es(f)})
                            })
                            .collect();
                        out.insert(k, Value::Array(flags));
                    }
                    _ => {
                        out.insert(k, to_v1(v));
                    }
                }
            }
            Value::Object(out)
        }
        other => other,
    }
}

/// El bloque de commits del entorno anterior se llama, en la v1, `<rama> (commits que mencionan el WI)`.
fn mention_blocks(v: &mut Value) {
    let Some(repos) = v.get_mut("repos").and_then(Value::as_array_mut) else { return };
    for r in repos {
        let mut renames = vec![];
        for b in r.get_mut("branches").and_then(Value::as_array_mut).into_iter().flatten() {
            if b.get("fromEnv").is_some() {
                let old = b["name"].as_str().unwrap().to_string();
                let new = wb_i18n::t("es", "promo.mentionBlock", &[("ref", &old)]);
                b["name"] = Value::String(new.clone());
                renames.push((old, new));
            }
        }
        let rename = |c: &mut Value| {
            for (old, new) in &renames {
                if c["branch"] == old.as_str() {
                    c["branch"] = Value::String(new.clone());
                }
            }
        };
        for b in r.get_mut("branches").and_then(Value::as_array_mut).into_iter().flatten() {
            b["commits"].as_array_mut().into_iter().flatten().for_each(rename);
        }
        r.get_mut("commits").and_then(Value::as_array_mut).into_iter().flatten().for_each(rename);
    }
}

fn scrub(v: Value, root: &str) -> Value {
    match v {
        Value::String(s) => Value::String(s.replace(root, "<ROOT>")),
        Value::Array(a) => Value::Array(a.into_iter().map(|x| scrub(x, root)).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, x)| (k, scrub(x, root))).collect()),
        other => other,
    }
}

#[track_caller]
fn assert_json(ours: &Value, expected: &Value, ctx: &str) {
    if ours != expected {
        panic!(
            "{ctx}\n--- ours ---\n{}\n--- expected ---\n{}",
            serde_json::to_string_pretty(ours).unwrap(),
            serde_json::to_string_pretty(expected).unwrap()
        );
    }
}

fn wis(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Candidatas como las de la v1 (ramas propias de QA y PROD que el repo de prueba no tiene, y
/// `origin/main` de respaldo), como datos.
fn v1_candidates() -> IndexMap<String, Vec<String>> {
    [("DEV", vec!["origin/development"]), ("QA", vec!["rel/qa"]), ("PROD", vec!["rel/main", "origin/main"])]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.into_iter().map(String::from).collect()))
        .collect()
}

#[test]
fn detect_and_default_envs() {
    let g = golden();
    let fx = Fixture::new("detect");
    let detected = wb_git::detect_repos(&fx.project.join("_work"), &v1_candidates());
    assert_json(&fx.v1(&detected), &g["detect_repos"], "detect_repos");
    let envs = wb_git::default_envs(&fx.app, &v1_candidates()).unwrap();
    assert_json(&json!(envs), &g["default_envs"], "default_envs");
    // Con las ramas del preset classic-es se detecta también QA (origin/qa).
    let wf = wb_core::Workflow::preset("classic-es").unwrap();
    let cands: IndexMap<String, Vec<String>> = wf
        .environments
        .iter()
        .filter_map(|e| wf.git.env_refs.get(&e.id).map(|v| (e.id.clone(), v.clone())))
        .collect();
    let envs = wb_git::default_envs(&fx.app, &cands).unwrap();
    assert_eq!(json!(envs), json!({"dev": "origin/development", "qa": "origin/qa", "prod": "origin/main"}));
}

#[test]
fn overview_and_cache() {
    let g = golden();
    let fx = Fixture::new("overview");
    let rules = rules();
    let repos = [fx.repo(&ENVS)];
    let panel = GitPanel::new(None).with_now(NOW);
    let ov = panel.overview(&repos, &wis(&WIS), &rules);
    assert_json(&fx.v1(&ov), &g["overview"], "overview");
    let one = panel.overview(&repos, &wis(&["555555"]), &rules);
    assert_json(&fx.v1(&one), &g["overview_one"], "overview_one");
    let missing = GitPanel::new(None).with_now(NOW).overview(
        &[fx.repo(&[ENVS[0], ("QA", "origin/no-existe"), ENVS[2]])],
        &wis(&["111111"]),
        &rules,
    );
    assert_json(&fx.v1(&missing), &g["overview_missing_env"], "overview_missing_env");
    let docs = RepoConfig {
        name: "docs".into(),
        path: fx.project.join("docs").display().to_string(),
        ..fx.repo(&ENVS)
    };
    let not_repo = GitPanel::new(None).with_now(NOW).overview(&[docs], &wis(&WIS), &rules);
    assert_json(&fx.v1(&not_repo), &g["overview_not_repo"], "overview_not_repo");

    // branch_diff de 111111 (tres commits propios) y de la rama mergeada por fast-forward (ninguno).
    let tip = |id: &str| ov.by_item[id][0].tip.sha.clone();
    let diff = panel.branch_diff(&repos[0], &tip("111111"), &rules, 400_000).unwrap();
    assert_json(&json!(diff), &g["branch_diff"]["ok"], "branch_diff");
    let ff = panel.branch_diff(&repos[0], &tip("999999"), &rules, 400_000).unwrap();
    assert_json(&json!(ff), &g["branch_diff_ff"]["ok"], "branch_diff_ff");

    // Mismo cálculo con el caché en disco caliente: tiene que dar lo mismo.
    assert_eq!(g["overview_warm_equals_cold"], true);
    let cache = fx.tmp.join("git_cache.json");
    let cold = GitPanel::new(Some(cache.clone())).with_now(NOW).overview(&repos, &wis(&WIS), &rules);
    assert!(cache.exists());
    let warm = GitPanel::new(Some(cache)).with_now(NOW).overview(&repos, &wis(&WIS), &rules);
    assert_json(&fx.v1(&warm), &fx.v1(&cold), "warm = cold");
    assert_json(&fx.v1(&warm), &g["overview"], "warm overview");
}

#[test]
fn promotion() {
    let g = golden();
    let fx = Fixture::new("promo");
    let rules = rules();
    let repos = [fx.repo(&ENVS)];
    let panel = GitPanel::new(None).with_now(NOW);
    for (case, ids, target) in [
        ("promotion_qa", &WIS[..], "QA"),
        ("promotion_prod", &WIS[..], "PROD"),
        ("promotion_qa_one", &["444444"][..], "QA"),
        ("promotion_qa_stacked", &["555555"][..], "QA"),
        ("promotion_qa_mention_only", &["333333"][..], "QA"),
    ] {
        let out = panel.promotion(&repos, &wis(ids), target, &rules);
        assert_json(&fx.v1(&out), &g[case], case);
    }
    let missing = panel.promotion(
        &[fx.repo(&[ENVS[0], ("QA", "origin/no-existe"), ENVS[2]])],
        &wis(&["111111"]),
        "QA",
        &rules,
    );
    assert_json(&fx.v1(&missing), &g["promotion_missing_target"], "promotion_missing_target");
}

#[test]
fn helpers() {
    let g = golden();
    let rules = rules();
    for case in g["helpers"]["wi_nums"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap_or("");
        let ours: Vec<String> = rules.ids_in_message(text).into_iter().collect();
        assert_eq!(json!(ours), case["wis"], "wi_nums {text:?}");
    }
    for case in g["helpers"]["cp_trailer"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        assert_eq!(json!(wb_git::cp_trailer(text)), case["shas"], "cp_trailer {text:?}");
    }
    for case in g["helpers"]["remote_web_url"].as_array().unwrap() {
        let Some(url) = case["url"].as_str() else { continue };
        let ours = wb_git::web_url(url).map(|(_, u)| u);
        if url.contains("github.com") {
            // Divergencia buscada (KNOWN_ISSUES #3): el port también arma la URL de GitHub y GitLab.
            assert_eq!(ours.as_deref(), Some("https://github.com/acme/app"));
            continue;
        }
        assert_eq!(json!(ours), case["web"], "remote_web_url {url}");
    }
    let fx = Fixture::new("helpers");
    assert_eq!(wb_git::remote_web_url(&fx.app, "noexiste"), None);
    assert_eq!(wb_git::remote_web_url(&fx.app, "origin"), None);
}

#[test]
fn watch_signature_changes_with_the_repo() {
    let fx = Fixture::new("watch");
    let repos = [fx.repo(&ENVS)];
    let a = wb_git::watch_signature(&repos);
    assert_eq!(a, wb_git::watch_signature(&repos));
    std::fs::write(fx.app.join(".git/FETCH_HEAD"), "x\n").unwrap();
    assert_ne!(a, wb_git::watch_signature(&repos));
}
