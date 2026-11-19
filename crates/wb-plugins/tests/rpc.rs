//! El runner contra un plugin real: este mismo ejecutable, copiado a la carpeta del plugin, hace de plugin
//! cuando lo lanza la app (que define `WORKBOARD_PLUGIN_PROTOCOL`); el modo de prueba sale de `mode.txt` en
//! la carpeta del plugin. Sin harness: `main` decide qué papel juega.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};
use wb_plugins::{Host, Plugins, call, sdk};

fn plugin_main(mode: &str) {
    if mode == "silent" {
        // No contesta nunca: el runner tiene que cortarlo por timeout.
        std::thread::sleep(std::time::Duration::from_secs(60));
        return;
    }
    if mode == "garbage" {
        println!("hola, no soy JSON");
        return;
    }
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut output = std::io::stdout();
    sdk::serve(&mut input, &mut output, |method, params, client| match method {
        "action/open" => {
            let item = client.request("host/item", json!({"name": params["context"]["items"][0]["name"]}))?;
            let denied = client.request("host/secret/get", json!({"key": "x"})).err();
            eprintln!("log line");
            Ok(json!({
                "type": "form",
                "title": format!("{} · {}", item["title"].as_str().unwrap_or(""), client.lang()),
                "fields": [{"id": "name", "type": "text", "label": "Name", "required": true}],
                "buttons": [{"id": "ok", "label": "OK", "primary": true}],
                "state": {"denied": denied},
            }))
        }
        "action/submit" => Ok(json!({"type": "close", "message": format!("hi {}", params["values"]["name"].as_str().unwrap_or(""))})),
        "action/fail" => Err("nope".into()),
        _ => Err("unknown method".into()),
    })
    .expect("serve");
}

struct FakeHost {
    calls: Vec<String>,
}

impl Host for FakeHost {
    fn request(&mut self, method: &str, params: &Json) -> Result<Json, String> {
        self.calls.push(method.to_string());
        match method {
            "host/item" => Ok(json!({"name": params["name"], "title": "Login"})),
            _ => Err("plugin.denied".into()),
        }
    }
}

fn install(base: &Path, folder: &str, manifest: Json) -> PathBuf {
    let dir = base.join("plugins").join(folder);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let exe = std::env::current_exe().unwrap();
    let name = if cfg!(windows) { "plugin.exe" } else { "plugin" };
    std::fs::copy(&exe, dir.join("bin").join(name)).unwrap();
    std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    dir
}

fn manifest(id: &str, mode: &str, timeout: u64) -> Json {
    json!({
        "id": id, "name": {"en": "Test", "es": "Prueba"}, "version": "1.0.0", "protocol": 1,
        "command": "bin/plugin", "args": [], "timeoutSeconds": timeout,
        "permissions": ["items.write"], "fields": ["release_manual"], "presets": ["presets/flow.json"],
        "actions": [{"id": "hello", "label": "Hello", "placements": ["item", "palette"], "when": {"kind": "item"}}],
        "_mode": mode,
    })
}

fn check(name: &str, ok: bool) {
    if ok {
        println!("test {name} ... ok");
    } else {
        println!("test {name} ... FAILED");
        std::process::exit(1);
    }
}

fn main() {
    if std::env::var_os("WORKBOARD_PLUGIN_PROTOCOL").is_some() {
        let mode = std::fs::read_to_string("mode.txt").unwrap_or_default();
        return plugin_main(mode.trim());
    }
    let base = std::env::temp_dir().join(format!("wb-plugins-rpc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);

    // Un manifiesto con una clave desconocida no se carga (deny_unknown_fields).
    install(&base, "bad", manifest("bad", "", 10));
    // Los demás, sin la clave de prueba: el modo va por variable de entorno.
    let strip = |mut m: Json| {
        m.as_object_mut().unwrap().remove("_mode");
        m
    };
    let ok_dir = install(&base, "ok", strip(manifest("ok", "", 20)));
    std::fs::create_dir_all(ok_dir.join("presets")).unwrap();
    let mut preset: Json =
        serde_json::from_str(include_str!("../../wb-core/presets/classic-es.json")).unwrap();
    preset["id"] = json!("from-plugin");
    std::fs::write(ok_dir.join("presets/flow.json"), preset.to_string()).unwrap();
    install(&base, "zz-dup", strip(manifest("ok", "", 20)));
    install(&base, "slow", strip(manifest("slow", "", 5)));

    let all = Plugins::load(&base);
    let by_folder = |f: &str| all.list.iter().find(|p| p.folder == f).unwrap();
    check(
        "unknown manifest keys are rejected",
        by_folder("bad").error.as_deref().is_some_and(|e| e.contains("_mode")),
    );
    check(
        "repeated ids are rejected",
        by_folder("zz-dup").error.as_deref().is_some_and(|e| e.contains("repeated")),
    );
    check("plugins start disabled", all.enabled().count() == 0);
    check("disabled presets do not resolve", wb_plugins::resolve_workflow(&base, "from-plugin").is_err());

    Plugins::set_enabled(&base, "ok", true).unwrap();
    Plugins::set_enabled(&base, "slow", true).unwrap();
    let all = Plugins::load(&base);
    check("enabled plugins are listed", all.enabled().count() == 2);
    check("plugin presets resolve when enabled", wb_plugins::resolve_workflow(&base, "from-plugin").is_ok());
    check("built-in presets still resolve", wb_plugins::resolve_workflow(&base, "classic-es").is_ok());
    let (ok, _) = all.get("ok").unwrap();

    let run = |plugin: &wb_plugins::Plugin, mode: &str, method: &str, params: Json| {
        std::fs::write(plugin.dir.join("mode.txt"), mode).unwrap();
        let mut host = FakeHost { calls: vec![] };
        let mut stderr = String::new();
        let out = call(plugin, "es", method, params, &mut host, &mut stderr);
        (out, host.calls, stderr)
    };
    let ctx = json!({"lang": "es", "root": "/x", "items": [{"name": "wi1-login", "kind": "item"}]});
    let (out, calls, stderr) = run(ok, "", "action/open", json!({"action": "hello", "context": ctx}));
    let out = out.expect("open");
    check("form step comes back", out["type"] == "form" && out["title"] == "Login · es");
    check("host requests are answered in order", calls == ["host/item", "host/secret/get"]);
    check("host errors reach the plugin", out["state"]["denied"] == "plugin.denied");
    check("stderr is captured", stderr.contains("log line"));
    let (out, _, _) = run(ok, "", "action/submit", json!({"action": "hello", "values": {"name": "Ana"}}));
    check("submit works", out.unwrap()["message"] == "hi Ana");
    let (out, _, _) = run(ok, "", "action/fail", json!({}));
    check(
        "plugin errors map to plugin.error.failed",
        out.unwrap_err() == wb_plugins::PluginError { code: "plugin.error.failed", detail: "nope".into() },
    );
    let (out, _, _) = run(ok, "garbage", "action/open", json!({}));
    check("non-JSON output is a protocol error", out.unwrap_err().code == "plugin.error.protocol");
    let (slow, _) = all.get("slow").unwrap();
    let started = std::time::Instant::now();
    let (out, _, _) = run(slow, "silent", "action/open", json!({}));
    check("silent plugins time out", out.unwrap_err().code == "plugin.error.timeout");
    check("timeout is honoured", started.elapsed() < std::time::Duration::from_secs(15));

    let _ = std::fs::remove_dir_all(&base);
    println!("test result: ok");
}
