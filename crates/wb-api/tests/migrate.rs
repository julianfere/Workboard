//! `workboard migrate-v1` contra una carpeta de config de la v1 armada en una carpeta temporal (mismo
//! formato que escribe la app Python). Los PATs se cifran en el momento con DPAPI de usuario (como la v1);
//! fuera de Windows, con un cifrado falso. Los secretos van a un almacén en memoria.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value as Json, json};
use wb_api::Api;
use wb_api::migrate::{self, Options, Outcome, Report};
use wb_platform::secrets::{MemorySecrets, SecretStore};

const PAT: &str = "fake-read-pat-0123456789";
const WRITE_PAT: &str = "fake-write-pat-9876543210";
const ORG: &str = "https://dev.azure.com/contoso";

/// Cifra como la v1: DPAPI de usuario en Windows; un cifrado falso en los demás sistemas.
fn protect(secret: &str) -> String {
    wb_platform::dpapi::protect(secret)
        .unwrap_or_else(|_| format!("fake:{}", secret.chars().rev().collect::<String>()))
}

fn unprotect(token: &str) -> io::Result<String> {
    match token.strip_prefix("fake:") {
        Some(rest) if !wb_platform::dpapi::SUPPORTED => Ok(rest.chars().rev().collect()),
        _ => wb_platform::dpapi::unprotect(token),
    }
}

struct Fx {
    base: PathBuf,
    v1: PathBuf,
    v2: PathBuf,
    root: PathBuf,
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(path: &Path) -> Json {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Una config de la v1 con todo lo que se migra (y algo de lo que no).
fn fixture(tag: &str) -> Fx {
    let base = std::env::temp_dir().join(format!("wb-migrate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("Proj").join("_work");
    write(&root.join("wi100-algo/README.md"), "---\nestado: plan\n---\n# Algo\n");
    let other = base.join("Other").join("_work");
    std::fs::create_dir_all(&other).unwrap();
    let repo = base.join("Proj").join("api");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let root = dunce::canonicalize(&root).unwrap();
    let other = dunce::canonicalize(&other).unwrap();
    let repo = dunce::canonicalize(&repo).unwrap();
    let (rs, os) = (root.display().to_string(), other.display().to_string());
    let key = |s: &str| if cfg!(windows) { s.to_lowercase() } else { s.to_string() };
    let v1 = base.join("v1");
    let v2 = base.join("v2");
    let config = json!({
        "root": rs,
        "recents": [rs, os, base.join("gone").display().to_string()],
        "aliases": {key(&rs): "Proyecto"},
        "prefs": {"theme": "dark", "view": "board", "hideClosed": true},
        "notesFolder": null,
        "agents": {"default": "copilot", "custom": {"label": "Mi agente", "command": "myagent", "args": "--id {session_id}", "resumeArgs": ""}},
        "git": {key(&rs): [{"name": "api", "path": repo.display().to_string(), "envs": {"DEV": "origin/develop", "QA": "origin/qa", "PROD": "origin/main"}}]},
        "ado": {
            "conn": {key(&rs): {"org": ORG, "project": "Shop"}},
            "pats": {ORG: protect(PAT)},
            "wpats": {ORG: protect(WRITE_PAT)},
            "procs": {format!("{ORG}|shop"): {
                "states": [
                    {"name": "New", "color": "b2b2b2", "category": "Proposed", "types": ["Bug"]},
                    {"name": "Active", "color": "007acc", "category": "InProgress", "types": ["Bug"]},
                    {"name": "Closed", "color": "339933", "category": "Completed", "types": ["Bug"]}
                ],
                "fetchedAt": 1_760_000_000.5,
                "map": {"New": "pendiente", "Active": "en-curso", "Closed": "prod", "Weird": "no-existe"},
                "mapSaved": true,
                "dateFields": [{"ref": "Custom.Pactada", "name": "Fecha pactada"}],
                "hasPriority": true,
                "hasEffort": false,
                "pactadaField": "Custom.Pactada",
                "numFields": [{"ref": "Custom.Horas", "name": "Horas"}],
                "horasField": ""
            }},
            "rm": {format!("{ORG}|shop"): {"repo": "db"}}
        }
    });
    write(&v1.join("config.json"), &serde_json::to_string_pretty(&config).unwrap());
    write(
        &v1.join("runs.json"),
        &json!({key(&rs): {"wi100-algo/MIGRATION_1.sql": {"dev": 1_760_000_100.0, "qa": 1_760_000_200.0}}})
            .to_string(),
    );
    write(
        &v1.join("state_log.jsonl"),
        &format!(
            "{}\n{}\n",
            json!({"ts": 1_760_000_000.0, "root": key(&rs), "wi": "wi100-algo", "de": "pendiente", "a": "plan", "session": null}),
            json!({"ts": 1_760_000_050.0, "root": key(&rs), "wi": "wi100-algo", "de": "plan", "a": "en-curso", "session": "s1"})
        ),
    );
    write(&v1.join("mcp_events.jsonl"), "{\"ts\": 1.0, \"tool\": \"workboard_set_estado\"}\n");
    let script = "print('hola')\n";
    write(&v1.join("tools/saludo/saludo.py"), script);
    write(
        &v1.join("tools/saludo/tool.json"),
        &json!({"nombre": "saludo", "archivo": "saludo.py", "interprete": "python", "descripcion": "Saluda",
                "uso": "", "origen": {}, "sha256": wb_sessions::tools::sha256_file(&v1.join("tools/saludo/saludo.py")).unwrap(), "promovida": 1.0})
        .to_string(),
    );
    write(&v1.join("tools/viejo/viejo.py"), "print(2)\n");
    write(
        &v1.join("tools/viejo/tool.json"),
        &json!({"nombre": "viejo", "archivo": "viejo.py", "interprete": "python", "sha256": "0".repeat(64)})
            .to_string(),
    );
    write(&v1.join("tools/_removed/x/tool.json"), "{}");
    write(&v1.join("inbox.md"), "# Bandeja\n\n## Pendientes\n\n- [ ] Llamar a Juan\n");
    write(&v1.join("notas/hallazgo.md"), "---\nwis: [100]\n---\n# Hallazgo\n");
    write(&v1.join("notas/sub/otra.md"), "# Otra\n");
    write(&v1.join("notas-borradas/vieja.md"), "# Vieja\n");
    write(&v1.join("backups/_work-abc123/wi100-algo/20261101-120000.md"), "---\nestado: pendiente\n---\n");
    let sess = json!({"sessions": [
        {"id": "s1", "nombre": "Algo", "raiz": rs, "cwd": root.parent().unwrap().display().to_string(),
         "wis": ["wi100-algo"], "estado": "activa", "modo": "plan", "agente": "claude", "plan": true,
         "modelo": "opus", "creada": "2026-11-01T10:00:00", "ultimaActividad": "2026-11-01T11:00:00",
         "forkDe": null, "repos": ["api"]},
        {"id": "s2", "nombre": "Vieja", "raiz": rs, "cwd": "", "wis": ["wi100-algo"], "estado": "cerrada",
         "modo": "default", "creada": "2026-10-01T10:00:00", "ultimaActividad": "2026-10-01T11:00:00", "forkDe": "s1", "repos": []}
    ]});
    write(&v1.join("sessions/Proj-_work-12345678.json"), &sess.to_string());
    write(
        &v1.join("sessions/s1.activity.jsonl"),
        "{\"ts\": 10.0, \"ev\": \"launch\", \"wis\": [\"wi100-algo\"]}\n{\"ts\": 20.0, \"ev\": \"exit\", \"wis\": [\"wi100-algo\"]}\n",
    );
    write(&v1.join("sessions/s1.events.jsonl"), "{\"ts\": 15.0, \"tool\": \"set_estado\"}\n");
    write(&v1.join("ado_cache.json"), "{}");
    write(&v1.join("workboard.log"), "x");
    write(&v1.join("ctx/s1/AGENTS.md"), "x");
    Fx { base, v1, v2, root }
}

fn opts(fx: &Fx) -> Options {
    Options {
        from: fx.v1.clone(),
        to: fx.v2.clone(),
        preset: "classic-es".into(),
        dry_run: false,
        force: false,
    }
}

fn run(o: &Options, secrets: &dyn SecretStore) -> Report {
    migrate::run(o, secrets, &unprotect).unwrap()
}

/// Todo el contenido de una carpeta (para comprobar que no cambió).
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                walk(&e.path(), out);
            } else {
                out.insert(e.path(), std::fs::read(e.path()).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, &mut out);
    out
}

fn outcome(r: &Report, item: &str) -> Outcome {
    let found = r.find(item);
    assert!(!found.is_empty(), "{item} not in the report");
    found[0].outcome
}

#[test]
fn migrates_everything_and_leaves_v1_untouched() {
    let fx = fixture("full");
    let before = snapshot(&fx.v1);
    let secrets = Arc::new(MemorySecrets::default());
    let r = run(&opts(&fx), secrets.as_ref());
    assert_eq!(snapshot(&fx.v1), before, "the v1 folder changed");
    assert!(!r.failed(), "{}", migrate::render(&r, "en"));
    assert_eq!(r.lang, "es");

    // settings.json
    let s = read(&fx.v2.join("settings.json"));
    let rs = fx.root.display().to_string();
    assert_eq!(s["workflow"], "classic-es");
    assert_eq!(s["lang"], "es");
    assert_eq!(s["theme"], "dark");
    assert_eq!(s["root"], rs.as_str());
    assert_eq!(s["recents"].as_array().unwrap().len(), 3);
    assert_eq!(s["recents"][0], rs.as_str());
    assert_eq!(s["notesFolder"], Json::Null);
    // El alias de la v1 (clave en minúsculas) queda con la grafía de la carpeta.
    assert_eq!(s["aliases"], json!({rs.as_str(): "Proyecto"}));
    assert_eq!(outcome(&r, "migrate.item.aliases"), Outcome::Done);
    assert!(!r.ignored.contains(&"migrate.ignored.aliases"));
    assert_eq!(s["agents"]["default"], "copilot");
    assert_eq!(s["agents"]["custom"]["command"], "myagent");
    assert_eq!(s["agents"]["custom"]["args"], "--id {session_id}");
    let repos = &s["git"]["repos"][rs.as_str()];
    assert_eq!(repos[0]["name"], "api");
    assert_eq!(repos[0]["envs"], json!({"dev": "origin/develop", "qa": "origin/qa", "prod": "origin/main"}));

    // trackers.json + secretos
    let t = read(&fx.v2.join("trackers.json"));
    let conns = t["connections"].as_object().unwrap();
    assert_eq!(conns.len(), 1);
    let conn = conns.values().next().unwrap();
    assert_eq!(conn, &json!({"kind": "azure-devops", "org": ORG, "project": "Shop"}));
    let proj = &t["projects"][format!("azure-devops|{ORG}|shop")];
    assert_eq!(proj["map"], json!({"Active": "en-curso", "Closed": "prod", "New": "pendiente"}));
    assert_eq!(proj["mapSaved"], true);
    assert_eq!(proj["states"][1]["suggested"], "active");
    assert_eq!(proj["dueField"], "Custom.Pactada");
    assert_eq!(proj["hoursField"], "");
    assert_eq!(r.find("migrate.item.project")[0].note, Some("migrate.note.mapDropped"));
    assert_eq!(secrets.get(&format!("azure-devops:read:{ORG}")).unwrap().as_deref(), Some(PAT));
    assert_eq!(secrets.get(&format!("azure-devops:write:{ORG}")).unwrap().as_deref(), Some(WRITE_PAT));

    // registros
    let runs = read(&fx.v2.join("runs.json"));
    let marks = runs.as_object().unwrap().values().next().unwrap();
    assert_eq!(marks["wi100-algo/MIGRATION_1.sql"]["qa"], 1_760_000_200.0);
    assert_eq!(std::fs::read_to_string(fx.v2.join("state_log.jsonl")).unwrap().lines().count(), 2);
    assert!(fx.v2.join("mcp_events.jsonl").is_file());
    assert!(fx.v2.join("tools/saludo/saludo.py").is_file());
    assert!(fx.v2.join("tools/viejo/tool.json").is_file());
    assert!(!fx.v2.join("tools/_removed").exists());
    let tools = r.find("migrate.item.tool");
    assert_eq!(
        tools.iter().find(|e| e.param("name") == Some("viejo")).unwrap().note,
        Some("migrate.note.toolHash")
    );
    assert_eq!(tools.iter().find(|e| e.param("name") == Some("saludo")).unwrap().note, None);
    assert!(std::fs::read_to_string(fx.v2.join("inbox.md")).unwrap().contains("Llamar a Juan"));
    assert!(fx.v2.join("notes/hallazgo.md").is_file());
    assert!(fx.v2.join("notes/sub/otra.md").is_file());
    assert!(fx.v2.join("backups/_work-abc123/wi100-algo/20261101-120000.md").is_file());

    // sesiones
    let reg = read(&fx.v2.join("sessions/Proj-_work-12345678.json"));
    let list = reg["sessions"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["name"], "Algo");
    assert_eq!(list[0]["items"], json!(["wi100-algo"]));
    assert_eq!(list[0]["state"], "paused");
    assert_eq!(list[0]["model"], "opus");
    assert_eq!(list[1]["state"], "closed");
    assert_eq!(list[1]["agent"], "claude");
    assert_eq!(list[1]["forkOf"], "s1");
    assert!(fx.v2.join("sessions/s1.activity.jsonl").is_file());
    assert!(fx.v2.join("sessions/s1.events.jsonl").is_file());

    // Lo ignorado se informa.
    for k in [
        "migrate.ignored.caches",
        "migrate.ignored.log",
        "migrate.ignored.sessionFiles",
        "migrate.ignored.prefs",
        "migrate.ignored.releaseManual",
        "migrate.ignored.deletedNotes",
        "migrate.ignored.removedTools",
    ] {
        assert!(r.ignored.contains(&k), "{k}");
    }

    // La app abre lo migrado: carpeta, flujo y conexión con sus dos credenciales.
    let api = Api::new(fx.v2.clone());
    api.set_tracker_backend(secrets.clone(), Arc::new(NoHttp));
    let cfg = api.dispatch("tracker_config", &json!({})).unwrap();
    assert_eq!(cfg["connected"], true);
    assert_eq!(cfg["hasSecret"], true);
    assert_eq!(cfg["hasWriteSecret"], true);
    assert_eq!(cfg["dueField"], "Custom.Pactada");
    assert_eq!(api.dispatch("workflow", &json!({})).unwrap()["id"], "classic-es");
    let items = api.dispatch("list_items", &json!({})).unwrap();
    assert!(items.to_string().contains("wi100-algo"));
}

struct NoHttp;

impl wb_tracker::Transport for NoHttp {
    fn send(&self, _: &wb_tracker::HttpRequest) -> Result<wb_tracker::HttpResponse, String> {
        Err("no network in tests".into())
    }
}

#[test]
fn dry_run_writes_nothing() {
    let fx = fixture("dry");
    let secrets = MemorySecrets::default();
    let r = run(&Options { dry_run: true, ..opts(&fx) }, &secrets);
    assert!(!fx.v2.exists(), "dry run created {}", fx.v2.display());
    assert_eq!(secrets.get(&format!("azure-devops:read:{ORG}")).unwrap(), None);
    assert_eq!(r.count(Outcome::Done), 0);
    assert!(r.count(Outcome::Planned) > 10);
    assert!(!r.failed());
    let text = migrate::render(&r, "es");
    assert!(text.contains("Simulación"));
}

#[test]
fn second_run_changes_nothing() {
    let fx = fixture("again");
    let secrets = MemorySecrets::default();
    run(&opts(&fx), &secrets);
    let after_first = snapshot(&fx.v2);
    let r = run(&opts(&fx), &secrets);
    assert_eq!(snapshot(&fx.v2), after_first);
    let changed: Vec<_> = r.entries.iter().filter(|e| e.outcome != Outcome::Same).map(|e| e.item).collect();
    assert!(changed.is_empty(), "{changed:?}");
}

#[test]
fn existing_values_are_kept_unless_forced() {
    let fx = fixture("force");
    let secrets = MemorySecrets::default();
    write(
        &fx.v2.join("settings.json"),
        &json!({"workflow": "default", "lang": "en", "theme": "light", "root": "C:/elsewhere", "recents": ["C:/elsewhere"],
                "aliases": {fx.root.display().to_string().to_uppercase(): "Mío"}})
            .to_string(),
    );
    secrets.set(&format!("azure-devops:read:{ORG}"), "another-token").unwrap();
    write(&fx.v2.join("inbox.md"), "# Bandeja\n\n## Pendientes\n\n- [ ] Otra cosa\n");
    write(&fx.v2.join("runs.json"), "{}");

    let r = run(&opts(&fx), &secrets);
    assert_eq!(outcome(&r, "migrate.item.lang"), Outcome::Kept);
    assert_eq!(outcome(&r, "migrate.item.theme"), Outcome::Kept);
    assert_eq!(outcome(&r, "migrate.item.root"), Outcome::Kept);
    assert_eq!(outcome(&r, "migrate.item.secretRead"), Outcome::Kept);
    assert_eq!(outcome(&r, "migrate.item.inbox"), Outcome::Kept);
    assert_eq!(outcome(&r, "migrate.item.aliases"), Outcome::Kept);
    // El preset por defecto cuenta como no elegido; los recientes se suman.
    assert_eq!(outcome(&r, "migrate.item.workflow"), Outcome::Done);
    assert_eq!(outcome(&r, "migrate.item.recents"), Outcome::Done);
    let s = read(&fx.v2.join("settings.json"));
    assert_eq!(s["lang"], "en");
    assert_eq!(s["root"], "C:/elsewhere");
    assert_eq!(s["recents"][0], "C:/elsewhere");
    assert_eq!(s["recents"].as_array().unwrap().len(), 4);
    assert_eq!(secrets.get(&format!("azure-devops:read:{ORG}")).unwrap().as_deref(), Some("another-token"));
    assert!(migrate::render(&r, "en").contains("--force"));

    let r = run(&Options { force: true, ..opts(&fx) }, &secrets);
    assert!(!r.failed());
    assert_eq!(outcome(&r, "migrate.item.root"), Outcome::Done);
    assert_eq!(outcome(&r, "migrate.item.secretRead"), Outcome::Done);
    let s = read(&fx.v2.join("settings.json"));
    assert_eq!(s["lang"], "es");
    assert_eq!(s["root"], fx.root.display().to_string().as_str());
    assert_eq!(s["aliases"].as_object().unwrap().len(), 1);
    assert_eq!(s["aliases"].as_object().unwrap().values().next().unwrap(), "Proyecto");
    assert_eq!(secrets.get(&format!("azure-devops:read:{ORG}")).unwrap().as_deref(), Some(PAT));
    assert!(std::fs::read_to_string(fx.v2.join("inbox.md")).unwrap().contains("Llamar a Juan"));
}

#[test]
fn logs_are_merged_without_duplicates() {
    let fx = fixture("merge");
    let secrets = MemorySecrets::default();
    write(
        &fx.v2.join("state_log.jsonl"),
        "{\"ts\": 1760000025.0, \"wi\": \"nuevo\", \"de\": \"\", \"a\": \"plan\"}\n",
    );
    let r = run(&opts(&fx), &secrets);
    assert_eq!(r.find("migrate.item.stateLog")[0].param("count"), Some("2"));
    let text = std::fs::read_to_string(fx.v2.join("state_log.jsonl")).unwrap();
    let wis: Vec<String> = text
        .lines()
        .map(|l| serde_json::from_str::<Json>(l).unwrap()["wi"].as_str().unwrap().to_string())
        .collect();
    // Ordenado por `ts`: la fila de la v2 queda entre las dos de la v1.
    assert_eq!(wis, ["wi100-algo", "nuevo", "wi100-algo"]);
    let r = run(&opts(&fx), &secrets);
    assert_eq!(outcome(&r, "migrate.item.stateLog"), Outcome::Same);
}

#[test]
fn bad_tokens_fail_without_leaking() {
    let fx = fixture("badpat");
    let mut cfg = read(&fx.v1.join("config.json"));
    cfg["ado"]["pats"][ORG] = json!("bm90LWEtcmVhbC1ibG9i");
    cfg["ado"]["wpats"] = json!({});
    write(&fx.v1.join("config.json"), &cfg.to_string());
    let secrets = MemorySecrets::default();
    let r = run(&opts(&fx), &secrets);
    assert!(r.failed());
    assert_eq!(outcome(&r, "migrate.item.secretRead"), Outcome::Failed);
    assert_eq!(r.find("migrate.item.secretRead")[0].note, Some("migrate.note.decrypt"));
    for lang in ["en", "es"] {
        let text = migrate::render(&r, lang);
        assert!(!text.contains("bm90LWEtcmVhbC1ibG9i"));
        assert!(!text.contains(PAT));
    }
    // El resto se migró igual.
    assert_eq!(outcome(&r, "migrate.item.connection"), Outcome::Done);
}

#[test]
fn report_has_no_secrets_nor_missing_texts() {
    let fx = fixture("render");
    let secrets = MemorySecrets::default();
    let key = regex::Regex::new(r"migrate\.[a-zA-Z]").unwrap();
    for dry_run in [true, false] {
        let r = run(&Options { dry_run, ..opts(&fx) }, &secrets);
        for lang in ["en", "es"] {
            let text = migrate::render(&r, lang);
            assert!(!key.is_match(&text), "untranslated key in:\n{text}");
            assert!(!text.contains(PAT) && !text.contains(WRITE_PAT));
            assert!(!text.contains('{'), "unfilled parameter in:\n{text}");
        }
    }
}

#[test]
fn errors_and_arguments() {
    let fx = fixture("errors");
    let secrets = MemorySecrets::default();
    let missing = Options { from: fx.base.join("nope"), ..opts(&fx) };
    assert_eq!(migrate::run(&missing, &secrets, &unprotect).unwrap_err().code, "migrate.error.noV1");
    let same = Options { to: fx.v1.clone(), ..opts(&fx) };
    assert_eq!(migrate::run(&same, &secrets, &unprotect).unwrap_err().code, "migrate.error.sameFolder");
    // Un preset de un plugin que no está habilitado en la v2 (el privado, por ejemplo).
    let tx = Options { preset: "my-flow".into(), ..opts(&fx) };
    assert_eq!(migrate::run(&tx, &secrets, &unprotect).unwrap_err().code, "migrate.error.preset");
    assert!(!fx.v2.exists());

    let args = |a: &[&str]| migrate::parse_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    let p =
        args(&["--from", "C:/v1", "--to=C:/v2", "--dry-run", "--force", "--preset", "my-flow"]).unwrap();
    assert_eq!(p.from.as_deref(), Some(Path::new("C:/v1")));
    assert_eq!(p.to.as_deref(), Some(Path::new("C:/v2")));
    assert!(p.dry_run && p.force && !p.help);
    assert_eq!(p.preset.as_deref(), Some("my-flow"));
    assert_eq!(args(&["--from"]).unwrap_err().code, "migrate.error.missingValue");
    assert_eq!(args(&["--what"]).unwrap_err().code, "migrate.error.unknownArg");
    assert!(args(&["-h"]).unwrap().help);
}

#[test]
fn cli_prints_the_report() {
    let fx = fixture("cli");
    let mut out = vec![];
    let code =
        migrate::cli(&["--help".to_string(), "--to".to_string(), fx.v2.display().to_string()], &mut out);
    assert_eq!(code, 0);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("migrate-v1"), "{text}");
    assert!(!fx.v2.exists());
}
