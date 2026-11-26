//! Un tracker de plugin de punta a punta: este ejecutable, copiado a la carpeta del plugin, hace de tracker
//! cuando lo lanza la app (`WORKBOARD_PLUGIN_PROTOCOL`). Sin harness, como `plugins.rs`.

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value as Json, json};
use wb_api::Api;
use wb_platform::secrets::MemorySecrets;
use wb_plugins::sdk;
use wb_tracker::{HttpRequest, HttpResponse, Transport};

const SECRET: &str = "tok-READ-123456";
const BAD: &str = "bad-SECRET-987654";

struct NoNet;
impl Transport for NoNet {
    fn send(&self, _: &HttpRequest) -> Result<HttpResponse, String> {
        Err("no network in tests".into())
    }
}

/// Lo que recibió el plugin, para que el test vea qué ids le llegaron (en su carpeta: es su cwd).
fn log_call(method: &str, params: &Json) {
    let mut p = params.clone();
    p["secretOk"] = json!(p["secret"] == SECRET);
    p.as_object_mut().unwrap().remove("secret");
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open("calls.jsonl").unwrap();
    writeln!(f, "{}", json!({"method": method, "params": p})).unwrap();
}

fn item(id: &str) -> Json {
    let (state, due, est) = match id {
        "100001" => ("Doing", "2026-11-20", json!(6.5)),
        "100002" => ("Done", "2026-11-21", json!(2)),
        _ => ("To Do", "2026-11-22", Json::Null),
    };
    json!({"state": state, "title": format!("Card {id}"), "type": "Task", "assigned": "Ana", "changed": "2026-11-01T10:00:00Z",
        "priority": 2, "due": due, "estimate": est, "completed": null})
}

/// El plugin: un tracker falso con tres tarjetas (`100001`…`100003`).
fn plugin_main() {
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut out = std::io::stdout();
    sdk::serve(&mut input, &mut out, |method, params, c| {
        if method.starts_with("action/") {
            let info = c.request("host/tracker", json!({})).unwrap_or_else(|e| json!({"error": e}));
            return Ok(json!({"type": "close", "state": {"tracker": info}}));
        }
        log_call(method, params);
        let secret = params["secret"].as_str().unwrap_or("");
        let conn = &params["connection"];
        if secret != SECRET {
            eprintln!("rejected {secret}");
            return Err(format!("tracker.auth: token {secret} rejected"));
        }
        if conn["org"] == "leak" {
            eprintln!("debug: using {secret}");
            return Err(format!("boom with {secret} inside"));
        }
        match method {
            "tracker/connect" => {
                let project = conn["project"].as_str().unwrap_or("");
                let project = project.trim_start_matches("https://fake.test/board/").to_uppercase();
                if project == "NOPE" {
                    return Err("tracker.notFound".into());
                }
                Ok(json!({"org": conn["org"], "project": project}))
            }
            "tracker/states" => Ok(json!([
                {"name": "To Do", "color": "", "category": "todo", "types": ["Task"], "suggested": "backlog"},
                {"name": "Doing", "color": "0055cc", "category": "doing", "types": ["Task"], "suggested": "active"},
                {"name": "Done", "color": "", "category": "done", "types": ["Task"], "suggested": "done"},
            ])),
            "tracker/fields" => Ok(json!([
                {"ref": "created", "name": "Created", "type": "dateTime"},
                {"ref": "duedate", "name": "Due date", "type": "date", "role": "due"},
                {"ref": "est", "name": "Estimate", "type": "number", "role": "hours"},
                {"ref": "prio", "name": "Priority", "type": "number", "role": "priority"},
                {"ref": "odd", "name": "Odd", "type": "text", "role": "nonsense"},
            ])),
            "tracker/workItems" => {
                let mut res = serde_json::Map::new();
                for id in params["ids"].as_array().unwrap().iter().filter_map(Json::as_str) {
                    if ["100001", "100002", "100003"].contains(&id) {
                        let mut it = item(id);
                        if params["request"]["text"] == true {
                            it["description"] = json!("Some **text**");
                            it["criteria"] = json!("- ok");
                        }
                        res.insert(id.to_string(), it);
                    }
                }
                res.insert("999999".into(), item("999999"));
                Ok(Json::Object(res))
            }
            "tracker/assignedToMe" => Ok(json!(["100003", 100001, "abc"])),
            "tracker/detail" => Ok(json!({"id": params["id"], "title": format!("Card {}", params["id"].as_str().unwrap()),
                "type": "Task", "state": "To Do", "description": "From the plugin", "criteria": ""})),
            "tracker/setState" => Ok(json!({"state": params["state"], "changed": "2026-11-26T00:00:00Z"})),
            _ => Err(format!("unknown method {method}")),
        }
    })
    .expect("serve");
}

fn tracker_decl(kind: &str) -> Json {
    json!({
        "kind": kind, "name": {"en": "Fake Jira", "es": "Jira falso"},
        "org": {"label": {"en": "Site", "es": "Sitio"}, "placeholder": "my-site", "required": false},
        "project": {"label": {"en": "Board", "es": "Tablero"}, "placeholder": "board URL"},
        "secret": {"label": "API token", "hint": {"en": "From your profile", "es": "Del perfil"}},
        "writeSecret": false,
        "itemUrl": "https://fake.test/{project}/{id}",
        "idPattern": "^[0-9]{6}$",
    })
}

fn install(cfg: &Path, folder: &str, id: &str, trackers: Json) -> PathBuf {
    let dir = cfg.join("plugins").join(folder);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let name = if cfg!(windows) { "plugin.exe" } else { "plugin" };
    std::fs::copy(std::env::current_exe().unwrap(), dir.join("bin").join(name)).unwrap();
    let manifest = json!({
        "id": id, "name": "Fake tracker", "protocol": 1, "command": "bin/plugin", "timeoutSeconds": 30,
        "permissions": ["tracker"], "trackers": trackers,
        "actions": [{"id": "info", "label": "Info", "placements": ["palette"]}],
    });
    std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    dir
}

fn check(name: &str, ok: bool, detail: impl std::fmt::Debug) {
    if ok {
        println!("test {name} ... ok");
    } else {
        println!("test {name} ... FAILED: {detail:?}");
        std::process::exit(1);
    }
}

fn readme(root: &Path, folder: &str, front: &str) {
    std::fs::create_dir_all(root.join(folder)).unwrap();
    std::fs::write(root.join(folder).join("README.md"), format!("---\n{front}---\n# {folder}\n")).unwrap();
}

fn calls(dir: &Path) -> Vec<Json> {
    std::fs::read_to_string(dir.join("calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn main() {
    if std::env::var_os("WORKBOARD_PLUGIN_PROTOCOL").is_some() {
        return plugin_main();
    }
    let base = std::env::temp_dir().join(format!("wb-api-tracker-plugin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("work");
    readme(&root, "wi100001-alpha", "");
    readme(&root, "wi100002-beta", "estado: pendiente\n");
    readme(&root, "wi1234-short", "estado: pendiente\n");
    let cfg = base.join("cfg");
    let dir = install(&cfg, "a-fake", "fake", json!([tracker_decl("fake-jira")]));
    install(&cfg, "z-dup", "dup", json!([tracker_decl("fake-jira")]));
    install(&cfg, "bad-url", "bad-url", json!([{"kind": "x", "name": "X", "itemUrl": "http://x/{id}"}]));
    install(
        &cfg,
        "bad-re",
        "bad-re",
        json!([{"kind": "y", "name": "Y", "itemUrl": "https://y/{id}", "idPattern": "(["}]),
    );
    install(
        &cfg,
        "builtin",
        "builtin",
        json!([{"kind": "azure-devops", "name": "Z", "itemUrl": "https://z/{id}"}]),
    );

    let api = Api::new(cfg.clone());
    let secrets = Arc::new(MemorySecrets::default());
    api.set_tracker_backend(secrets.clone(), Arc::new(NoNet));
    api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es", "lang": "es"}})).unwrap();
    api.dispatch("open_root", &json!({"path": root})).unwrap();

    // Manifiestos.
    let list = api.dispatch("plugins", &json!({})).unwrap();
    let by_folder =
        |f: &str| list["plugins"].as_array().unwrap().iter().find(|p| p["folder"] == f).unwrap().clone();
    check(
        "http itemUrl is rejected",
        by_folder("bad-url")["error"].as_str().is_some_and(|e| e.contains("itemUrl")),
        &list,
    );
    check(
        "bad idPattern is rejected",
        by_folder("bad-re")["error"].as_str().is_some_and(|e| e.contains("idPattern")),
        &list,
    );
    check("built-in kinds are reserved", by_folder("builtin")["error"].as_str().is_some(), &list);
    check(
        "plugins list the trackers they add",
        by_folder("a-fake")["trackers"] == json!([{"kind": "fake-jira", "name": "Jira falso"}]),
        &list,
    );

    let cfg0 = api.dispatch("tracker_config", &json!({})).unwrap();
    check(
        "disabled plugin trackers are not offered",
        cfg0["kinds"].as_array().unwrap().len() == 1,
        &cfg0["kinds"],
    );
    check(
        "azure stays the built-in default",
        cfg0["kinds"][0]["builtin"] == true && cfg0["kind"] == "azure-devops",
        &cfg0,
    );

    api.dispatch("plugin_enable", &json!({"id": "fake", "enabled": true})).unwrap();
    let list = api.dispatch("plugin_enable", &json!({"id": "dup", "enabled": true})).unwrap();
    let dup = list["plugins"].as_array().unwrap().iter().find(|p| p["id"] == "dup").unwrap().clone();
    check(
        "a kind clash disables the later tracker",
        dup["trackerErrors"]
            == json!([{"code": "plugin.trackerClash", "params": {"kind": "fake-jira", "plugin": "fake"}}]),
        &dup,
    );

    let cfg1 = api.dispatch("tracker_config", &json!({})).unwrap();
    let kinds = cfg1["kinds"].as_array().unwrap();
    check("the plugin kind is offered once", kinds.len() == 2 && kinds[1]["kind"] == "fake-jira", kinds);
    check(
        "the connect form comes from the manifest",
        kinds[1]["plugin"] == "fake"
            && kinds[1]["name"] == json!({"en": "Fake Jira", "es": "Jira falso"})
            && kinds[1]["org"]
                == json!({"label": {"en": "Site", "es": "Sitio"}, "placeholder": "my-site", "required": false})
            && kinds[1]["project"]["label"] == json!({"en": "Board", "es": "Tablero"})
            && kinds[1]["secret"]
                == json!({"label": "API token", "hint": {"en": "From your profile", "es": "Del perfil"}})
            && kinds[1]["writeSecret"] == false,
        &kinds[1],
    );

    // Conexión.
    let err = api
        .dispatch("tracker_connect", &json!({"kind": "fake-jira", "org": "", "project": "p", "secret": BAD}))
        .unwrap_err();
    let text = serde_json::to_string(&err).unwrap();
    check("a rejected credential maps to tracker.pluginAuth", err.code == "tracker.pluginAuth", &err);
    check("the secret never appears in errors", !text.contains(BAD) && !text.contains("987654"), &text);
    let log = std::fs::read_to_string(cfg.join("plugins.log")).unwrap_or_default();
    check("the secret never reaches plugins.log", log.contains("rejected ***") && !log.contains(BAD), &log);
    let err = api
        .dispatch("tracker_connect", &json!({"kind": "fake-jira", "project": "nope", "secret": SECRET}))
        .unwrap_err();
    check("tracker.notFound maps to projectNotFound", err.code == "tracker.projectNotFound", &err);
    let err = api.dispatch("tracker_connect", &json!({"kind": "fake-jira", "project": " "})).unwrap_err();
    check("the project is required", err.code == "tracker.noProject", &err);
    let err = api.dispatch("tracker_connect", &json!({"kind": "nope", "project": "x"})).unwrap_err();
    check("unknown kinds are refused", err.code == "tracker.unknownKind", &err);

    let c = api
        .dispatch(
            "tracker_connect",
            &json!({"kind": "fake-jira", "org": " ", "project": "https://fake.test/board/abc", "secret": SECRET}),
        )
        .unwrap();
    check(
        "tracker/connect normalizes the connection",
        c["connected"] == true && c["kind"] == "fake-jira" && c["project"] == "ABC" && c["org"] == "",
        &c,
    );
    check(
        "source is the plugin tracker's name",
        c["source"] == "Jira falso" && c["kindMissing"] == false,
        &c,
    );
    check(
        "without writeSecret the read secret writes",
        c["writeSecret"] == false && c["hasWriteSecret"] == true,
        &c,
    );
    let again = api.dispatch("tracker_connect", &json!({"kind": "fake-jira", "project": "abc"})).unwrap();
    check("reconnecting reuses the stored secret", again["hasSecret"] == true, &again);

    // Estados y campos.
    let f = api.dispatch("tracker_fetch", &json!({})).unwrap();
    check("states come from the plugin", f["states"].as_array().unwrap().len() == 3, &f["states"]);
    check("suggested categories seed the mapping", f["suggested"]["Doing"] == "en-curso", &f["suggested"]);
    check("roles preselect the due field", f["dueField"] == "duedate", &f);
    check("roles preselect the hours field", f["hoursField"] == "est", &f);
    check("the priority role enables priority", f["hasPriority"] == true && f["hasEffort"] == false, &f);
    check(
        "dates and numbers are told apart",
        f["dateFields"].as_array().unwrap().len() == 2 && f["numFields"].as_array().unwrap().len() == 2,
        &f,
    );
    api.dispatch(
        "tracker_set_map",
        &json!({"map": {"To Do": "pendiente", "Doing": "en-curso", "Done": "prod"}}),
    )
    .unwrap();

    // Sync.
    let s = api.dispatch("tracker_sync", &json!({})).unwrap();
    let cache = &s["cache"];
    check(
        "sync caches the plugin's items",
        cache["items"]["100001"]["state"] == "Doing" && cache["items"]["100001"]["estimate"] == 6.5,
        cache,
    );
    check(
        "ids outside idPattern are not requested",
        cache["requested"] == json!(["100001", "100002"]),
        cache,
    );
    check("items the plugin did not ask for are dropped", cache["items"].get("999999").is_none(), cache);
    check("the cache keeps the display name", cache["source"] == "Jira falso", cache);
    let sent: Vec<Json> = calls(&dir).into_iter().filter(|c| c["method"] == "tracker/workItems").collect();
    check(
        "ids outside idPattern are never sent",
        sent.iter().all(|c| !c["params"]["ids"].to_string().contains("1234\"")),
        &sent,
    );
    check(
        "workItems gets the chosen fields",
        sent.last().unwrap()["params"]["request"]
            == json!({"priority": true, "due": "duedate", "hours": "est", "effort": false, "text": false, "extra": []}),
        &sent,
    );
    let p = api.dispatch("tracker_preview", &json!({"includeDiffer": true})).unwrap();
    let rows = p["rows"].as_array().unwrap();
    let row = |n: &str| rows.iter().find(|r| r["name"] == n).cloned().unwrap_or(Json::Null);
    check("preview proposes from the plugin states", row("wi100001-alpha")["status"] == "en-curso", &p);
    check("preview advances by the mapping", row("wi100002-beta")["status"] == "prod", &p);
    check("skipped ids are not reported as missing", p["notFound"] == json!([]), &p);

    // Asignados, detalle y cambio de estado.
    let a = api.dispatch("tracker_assigned", &json!({"period": "2026-11"})).unwrap();
    let ids: Vec<&str> = a["items"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    check("assigned-to-me keeps the plugin's ids in order", ids == ["100003", "100001"], &a);
    check("assigned rows mark existing folders", a["items"][1]["existing"] == json!(["wi100001-alpha"]), &a);
    let asked = calls(&dir).into_iter().rfind(|c| c["method"] == "tracker/assignedToMe").unwrap();
    check(
        "assignedToMe gets the period and the closed states",
        asked["params"]["dueField"] == "duedate"
            && asked["params"]["start"] == "2026-11-20"
            && asked["params"]["exclude"] == json!(["Done"]),
        &asked,
    );
    let d = api.dispatch("tracker_item", &json!({"id": "100003"})).unwrap();
    check(
        "detail comes from the plugin",
        d["fromTracker"] == true && d["title"] == "Card 100003" && d["source"] == "Jira falso",
        &d,
    );
    let d = api.dispatch("tracker_item", &json!({"id": "1234"})).unwrap();
    check("ids outside idPattern stay manual", d["fromTracker"] == false, &d);
    let imp = api.dispatch("tracker_import", &json!({"ids": ["100003"]})).unwrap();
    check("import creates the folder", imp["done"].as_array().unwrap().len() == 1, &imp);
    let r = api
        .dispatch(
            "tracker_push_states",
            &json!({"changes": [{"id": "100001", "state": "Done"}, {"id": "1234", "state": "Done"}]}),
        )
        .unwrap();
    check("push writes through the plugin", r["done"] == json!([{"id": "100001", "state": "Done"}]), &r);
    check("push refuses ids outside idPattern", r["errors"][0]["error"]["code"] == "tracker.badId", &r);
    let set = calls(&dir).into_iter().rfind(|c| c["method"] == "tracker/setState").unwrap();
    check("setState uses the read secret without writeSecret", set["params"]["secretOk"] == true, &set);
    let cfg2 = api.dispatch("tracker_config", &json!({})).unwrap();
    check("push updates the cache", cfg2["cache"]["items"]["100001"]["state"] == "Done", &cfg2["cache"]);
    let err = api.dispatch("tracker_open", &json!({"id": "1234"})).unwrap_err();
    check("open refuses ids outside idPattern", err.code == "tracker.badId", &err);

    // host/tracker con un tracker de plugin.
    let step =
        api.dispatch("plugin_action", &json!({"plugin": "fake", "action": "info", "items": []})).unwrap();
    check(
        "host/tracker describes plugin connections",
        step["state"]["tracker"]
            == json!({"kind": "fake-jira", "source": "Jira falso", "org": "", "project": "ABC"}),
        &step,
    );

    // El secreto nunca sale en un error del plugin ni en su stderr.
    let mut trackers: Json =
        serde_json::from_str(&std::fs::read_to_string(cfg.join("trackers.json")).unwrap()).unwrap();
    let key = trackers["connections"].as_object().unwrap().keys().next().unwrap().clone();
    trackers["connections"][&key]["org"] = json!("leak");
    std::fs::write(cfg.join("trackers.json"), trackers.to_string()).unwrap();
    use wb_platform::secrets::SecretStore;
    secrets.set("fake-jira:read:leak", SECRET).unwrap();
    let err = api.dispatch("tracker_fetch", &json!({})).unwrap_err();
    let text = serde_json::to_string(&err).unwrap();
    check(
        "other plugin errors keep their message",
        err.code == "tracker.pluginError" && text.contains("boom with ***"),
        &text,
    );
    check("the secret is scrubbed from plugin errors", !text.contains(SECRET), &text);
    let log = std::fs::read_to_string(cfg.join("plugins.log")).unwrap_or_default();
    check("the secret is scrubbed from stderr", log.contains("using ***") && !log.contains(SECRET), &log);
    trackers["connections"][&key]["org"] = json!("");
    std::fs::write(cfg.join("trackers.json"), trackers.to_string()).unwrap();

    // Plugin deshabilitado: la conexión y la caché quedan; todo responde tracker.pluginMissing.
    api.dispatch("plugin_enable", &json!({"id": "dup", "enabled": false})).unwrap();
    api.dispatch("plugin_enable", &json!({"id": "fake", "enabled": false})).unwrap();
    let m = api.dispatch("tracker_config", &json!({})).unwrap();
    check(
        "a missing plugin keeps the connection and cache",
        m["connected"] == true && m["kindMissing"] == true && m["project"] == "ABC" && !m["cache"].is_null(),
        &m,
    );
    check("a missing plugin keeps the mapping", m["map"]["Doing"] == "en-curso", &m["map"]);
    for cmd in ["tracker_sync", "tracker_fetch"] {
        let err = api.dispatch(cmd, &json!({})).unwrap_err();
        check(&format!("{cmd} answers tracker.pluginMissing"), err.code == "tracker.pluginMissing", &err);
    }
    let err = api.dispatch("tracker_assigned", &json!({"period": "2026-11"})).unwrap_err();
    check("assigned answers tracker.pluginMissing", err.code == "tracker.pluginMissing", &err);
    let err = api.dispatch("tracker_open", &json!({"id": "100001"})).unwrap_err();
    check("open answers tracker.pluginMissing", err.code == "tracker.pluginMissing", &err);
    let d = api.dispatch("tracker_item", &json!({"id": "100002"})).unwrap();
    check(
        "new item falls back to manual",
        d["fromTracker"] == false && d["error"]["code"] == "tracker.pluginMissing",
        &d,
    );
    let r = api
        .dispatch("tracker_push_states", &json!({"changes": [{"id": "100001", "state": "Done"}]}))
        .unwrap_err();
    check("push answers tracker.pluginMissing", r.code == "tracker.pluginMissing", &r);
    api.dispatch("plugin_enable", &json!({"id": "fake", "enabled": true})).unwrap();
    let back = api.dispatch("tracker_sync", &json!({})).unwrap();
    check(
        "the connection works again once enabled",
        back["kindMissing"] == false && back["connected"] == true,
        &back,
    );

    // Azure sigue igual.
    let az = api.dispatch("tracker_disconnect", &json!({})).unwrap();
    check("disconnect goes back to azure", az["connected"] == false && az["source"] == "Azure DevOps", &az);

    api.shutdown();
    let _ = std::fs::remove_dir_all(&base);
    println!("test result: ok");
}
