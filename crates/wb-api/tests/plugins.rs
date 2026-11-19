//! Plugins de punta a punta: este ejecutable, copiado a la carpeta del plugin, hace de plugin cuando lo
//! lanza la app (`WORKBOARD_PLUGIN_PROTOCOL`). Sin harness: `main` decide qué papel juega.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value as Json, json};
use wb_api::Api;
use wb_platform::secrets::MemorySecrets;
use wb_plugins::sdk;
use wb_tracker::{HttpRequest, HttpResponse, Transport};

struct NoNet;
impl Transport for NoNet {
    fn send(&self, _: &HttpRequest) -> Result<HttpResponse, String> {
        Err("no network in tests".into())
    }
}

/// El plugin: cada botón prueba una parte del host.
fn plugin_main() {
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut out = std::io::stdout();
    sdk::serve(&mut input, &mut out, |method, params, c| {
        let name = params["context"]["items"][0]["name"].as_str().unwrap_or("").to_string();
        let r = |c: &mut sdk::Client, m: &str, p: Json| c.request(m, p).unwrap_or_else(|e| json!({"error": e}));
        match (method, params["button"].as_str()) {
            ("action/open", _) => {
                let item = r(c, "host/item", json!({"name": name}));
                let sql = item["files"]
                    .as_array()
                    .and_then(|f| f.iter().find(|f| f["name"] == "MIGRATION_WI111111_sesiones.sql"))
                    .map(|f| f["path"].clone())
                    .unwrap_or(Json::Null);
                let file = r(c, "host/readFile", json!({"path": sql}));
                let outside = r(c, "host/readFile", json!({"path": "../secret.txt"}));
                r(c, "host/secret/set", json!({"key": "pat", "value": "s3cret"}));
                let secret = r(c, "host/secret/get", json!({"key": "pat"}));
                r(c, "host/data/set", json!({"data": {"n": 1}}));
                let data = r(c, "host/data/get", json!({}));
                let tracker = r(c, "host/tracker", json!({}));
                let wf = r(c, "host/workflow", json!({}));
                Ok(json!({
                    "type": "form", "title": item["title"], "fields": [{"id": "x", "type": "text", "label": "X"}],
                    "state": {
                        "status": item["status"], "fileStart": file["text"].as_str().map(|t| t.chars().take(20).collect::<String>()),
                        "outside": outside, "secret": secret, "data": data, "tracker": tracker, "wf": wf["id"],
                    },
                }))
            }
            ("action/submit", Some("write")) => {
                let ok = r(c, "host/updateItem", json!({"name": name, "fields": {"estado": "en-curso", "release_manual": "4242"}}));
                let bad_state = r(c, "host/updateItem", json!({"name": name, "fields": {"estado": "nope"}}));
                let not_declared = r(c, "host/updateItem", json!({"name": name, "fields": {"otra": "x"}}));
                let multiline = r(c, "host/updateItem", json!({"name": name, "fields": {"release_manual": "a\nb"}}));
                Ok(json!({"type": "result", "title": "t", "ok": true, "steps": [],
                    "links": [{"label": "ok", "url": "https://x.test/1"}, {"label": "bad", "url": "http://x.test"}, {"label": "js", "url": "javascript:alert(1)"}],
                    "state": {"ok": ok["status"], "badState": bad_state, "notDeclared": not_declared, "multiline": multiline}}))
            }
            ("action/submit", Some("session")) => Ok(json!({"type": "session", "items": [name], "name": "From plugin", "mode": "acceptEdits", "prompt": "hola"})),
            ("action/submit", Some("bad")) => Ok(json!({"type": "form", "fields": [{"id": "a", "type": "script"}]})),
            _ => Ok(json!({"type": "close", "message": "bye"})),
        }
    })
    .expect("serve");
}

fn copy_dir(src: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        if e.path().is_dir() {
            copy_dir(&e.path(), &dest.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), dest.join(e.file_name())).unwrap();
        }
    }
}

fn install(cfg: &Path, id: &str, permissions: Json) -> PathBuf {
    let dir = cfg.join("plugins").join(id);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let name = if cfg!(windows) { "plugin.exe" } else { "plugin" };
    std::fs::copy(std::env::current_exe().unwrap(), dir.join("bin").join(name)).unwrap();
    let mut preset: Json =
        serde_json::from_str(include_str!("../../wb-core/presets/classic-es.json")).unwrap();
    preset["id"] = json!(format!("{id}-flow"));
    std::fs::create_dir_all(dir.join("presets")).unwrap();
    std::fs::write(dir.join("presets/flow.json"), preset.to_string()).unwrap();
    let manifest = json!({
        "id": id, "name": {"en": "Test plugin", "es": "Plugin de prueba"}, "protocol": 1, "command": "bin/plugin",
        "timeoutSeconds": 30, "permissions": permissions, "fields": ["release_manual"], "presets": ["presets/flow.json"],
        "actions": [
            {"id": "touch", "label": {"en": "Touch", "es": "Tocar"}, "placements": ["item"], "when": {"kind": "item", "hasIds": true}},
            {"id": "global", "label": "Global", "placements": ["palette", "settings"]},
        ],
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

fn main() {
    if std::env::var_os("WORKBOARD_PLUGIN_PROTOCOL").is_some() {
        return plugin_main();
    }
    let base = std::env::temp_dir().join(format!("wb-api-plugins-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("proyecto/_work");
    copy_dir(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/fixtures/work"), &root);
    std::fs::write(base.join("proyecto/secret.txt"), "no").unwrap();
    let cfg = base.join("cfg");
    install(&cfg, "full", json!(["items.write", "files.read", "secrets", "tracker"]));
    install(&cfg, "bare", json!([]));

    let api = Api::new(cfg.clone());
    api.set_tracker_backend(Arc::new(MemorySecrets::default()), Arc::new(NoNet));
    let exe = std::env::current_exe().unwrap();
    api.set_session_bins(exe.clone(), exe);
    api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es", "lang": "es"}})).unwrap();
    api.dispatch("open_root", &json!({"path": root})).unwrap();
    let (cmd, args) =
        if cfg!(windows) { ("cmd", "/c echo {session_id}") } else { ("sh", "-c \"echo {session_id}\"") };
    api.dispatch("set_agents", &json!({"default": "custom", "custom": {"command": cmd, "args": args}}))
        .unwrap();

    let list = api.dispatch("plugins", &json!({})).unwrap();
    check(
        "plugins are listed disabled",
        list["plugins"].as_array().unwrap().iter().all(|p| p["enabled"] == false),
        &list,
    );
    check("labels follow the app language", list["plugins"][1]["name"] == "Plugin de prueba", &list);
    let run = |plugin: &str, action: &str, items: Json, button: Option<&str>| {
        let mut args = json!({"plugin": plugin, "action": action, "items": items, "cols": 100, "rows": 30});
        if let Some(b) = button {
            args["button"] = json!(b);
            args["values"] = json!({});
            args["state"] = json!({});
        }
        api.dispatch("plugin_action", &args)
    };
    let item = json!(["wi111111-login-con-sso"]);
    check(
        "disabled plugins cannot run",
        run("full", "touch", item.clone(), None).unwrap_err().code == "plugin.notFound",
        (),
    );
    let presets = api.dispatch("workflow_presets", &json!({})).unwrap();
    check("disabled plugin presets are hidden", !presets.to_string().contains("full-flow"), &presets);

    api.dispatch("plugin_enable", &json!({"id": "full", "enabled": true})).unwrap();
    api.dispatch("plugin_enable", &json!({"id": "bare", "enabled": true})).unwrap();
    let presets = api.dispatch("workflow_presets", &json!({})).unwrap();
    check("enabled plugin presets are listed", presets.to_string().contains("\"plugin\":\"full\""), &presets);

    let step = run("full", "touch", item.clone(), None).unwrap();
    let st = &step["state"];
    check("open returns the form", step["type"] == "form" && step["title"] == "Login con SSO", &step);
    check("host/item gives the status", st["status"] == "en-curso", st);
    check("host/readFile reads item files", st["fileStart"].as_str().is_some_and(|t| !t.is_empty()), st);
    check("host/readFile refuses files outside items", st["outside"]["error"].as_str().is_some(), st);
    check("secrets round-trip in the store", st["secret"]["value"] == "s3cret", st);
    check("plugin data round-trips", st["data"] == json!({"n": 1}), st);
    check("tracker info is null without a connection", st["tracker"].is_null(), st);
    check("host/workflow gives the active preset", st["wf"] == "classic-es", st);

    let bare = run("bare", "touch", item.clone(), None).unwrap();
    let st = &bare["state"];
    check(
        "files.read is enforced",
        st["fileStart"].is_null() && st["outside"]["error"] == "plugin.denied",
        st,
    );
    check("secrets is enforced", st["secret"]["error"] == "plugin.denied", st);
    check("tracker is enforced", st["tracker"]["error"] == "plugin.denied", st);
    check("secrets are namespaced per plugin", st["secret"]["value"].is_null(), st);

    let res = run("full", "touch", item.clone(), Some("write")).unwrap();
    let st = &res["state"];
    check("updateItem writes workflow and declared fields", st["ok"] == "en-curso", st);
    let readme = std::fs::read_to_string(root.join("wi111111-login-con-sso/README.md")).unwrap();
    check("release_manual lands in the README", readme.contains("release_manual: 4242"), &readme);
    check(
        "workflow fields are validated",
        st["badState"]["error"].as_str().is_some_and(|e| e.starts_with("core.")),
        st,
    );
    check("undeclared fields are denied", st["notDeclared"]["error"] == "plugin.denied", st);
    check("multiline values are refused", st["multiline"]["error"].as_str().is_some(), st);
    check("only https links survive", res["links"].as_array().unwrap().len() == 1, &res["links"]);
    let bare_write = run("bare", "touch", item.clone(), Some("write")).unwrap();
    check(
        "items.write is enforced",
        bare_write["state"]["notDeclared"]["error"] == "plugin.denied" && bare_write["state"]["ok"].is_null(),
        &bare_write,
    );

    let sess = run("full", "touch", item.clone(), Some("session")).unwrap();
    check(
        "session steps open a session",
        sess["type"] == "session"
            && sess["session"]["name"] == "From plugin"
            && sess["session"]["mode"] == "acceptEdits",
        &sess,
    );
    let bad = run("full", "touch", item.clone(), Some("bad")).unwrap_err();
    check("invalid steps are rejected", bad.code == "plugin.error.badStep", &bad);
    let wrong = run("full", "touch", json!(["grp-checkout"]), None).unwrap_err();
    check("when is checked against the items", wrong.code == "plugin.notForItem", &wrong);
    let global = run("full", "global", json!([]), Some("x")).unwrap();
    check("palette actions run without items", global["type"] == "close", &global);

    api.dispatch("set_settings", &json!({"settings": {"workflow": "full-flow", "lang": "es", "root": root}}))
        .unwrap();
    check(
        "plugin presets can be chosen",
        api.dispatch("workflow", &json!({})).unwrap()["id"] == "full-flow",
        (),
    );
    let busy = api.dispatch("plugin_enable", &json!({"id": "full", "enabled": false})).unwrap_err();
    check("the plugin of the active preset cannot be disabled", busy.code == "plugin.presetInUse", &busy);

    api.shutdown();
    let _ = std::fs::remove_dir_all(&base);
    println!("test result: ok");
}
