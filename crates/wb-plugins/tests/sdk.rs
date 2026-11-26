//! Los SDKs de `sdk/` contra el runner real (`rpc::call`) y `workboard plugin new|check`. Los que necesitan
//! `python`/`py` o `node` pasan con un aviso si no están en el PATH. Todo en carpetas temporales.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value as Json, json};
use wb_plugins::cli::{self, Lang, NewOptions};
use wb_plugins::{Host, Plugins, call};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wb-plugins-sdk-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// El primero de `cmds` que está en el PATH y corre `--version`.
fn find(cmds: &[&'static str]) -> Option<&'static str> {
    let path = std::env::var("PATH").unwrap_or_default();
    cmds.iter().copied().find(|c| {
        wb_platform::which(c, &path).is_some()
            && Command::new(c)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    })
}

fn python() -> Option<&'static str> {
    let found = find(&["python", "py", "python3"]);
    if found.is_none() {
        eprintln!("note: python is not on the PATH, skipping");
    }
    found
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

const PLUGIN_PY: &str = r#"
from workboard_plugin import HostError, Plugin, TrackerError, form, text

plugin = Plugin()

@plugin.action("greet")
def greet(ctx):
    item = ctx.host.item(ctx.items[0]["name"])
    try:
        ctx.host.secret_get("x")
        denied = None
    except HostError as e:
        denied = e.code
    print("to the log")
    return form("%s · %s" % (item["title"], ctx.lang), [text("name", "Name", required=True)],
                buttons=[("ok", "OK")], state={"denied": denied})

@greet.submit
def submit(ctx, button, values, state):
    raise ValueError("bad %s" % values["name"])

@plugin.tracker("fake")
class Fake:
    def states(self, conn, secret):
        return [{"name": "Doing", "suggested": "active", "project": conn.project}]

    def connect(self, conn, secret):
        raise TrackerError("tracker.auth", "rejected " + secret)

plugin.run()
"#;

#[test]
fn python_sdk_through_the_runner() {
    let Some(py) = python() else { return };
    let config = temp("py-runner");
    let dir = config.join("plugins").join("sdk-py");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("workboard_plugin.py"), cli::PYTHON_SDK).unwrap();
    std::fs::write(dir.join("plugin.py"), PLUGIN_PY).unwrap();
    let manifest = json!({
        "id": "sdk-py", "name": "SDK", "version": "1.0.0", "protocol": 1, "command": py, "args": ["plugin.py"],
        "timeoutSeconds": 30,
        "actions": [{"id": "greet", "label": "Greet", "placements": ["item"]}],
    });
    std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    let all = Plugins::load(&config);
    let plugin = all.list.iter().find(|p| p.folder == "sdk-py").unwrap();
    assert!(plugin.error.is_none(), "{:?}", plugin.error);

    let run = |method: &str, params: Json| {
        let mut host = FakeHost { calls: vec![] };
        let mut stderr = String::new();
        let out = call(plugin, "es", method, params, &mut host, &mut stderr);
        (out, host.calls, stderr)
    };
    let ctx = json!({"lang": "es", "root": "/x", "placement": "item", "items": [{"name": "wi1-login"}]});
    let (out, calls, stderr) = run("action/open", json!({"action": "greet", "context": ctx}));
    let step = out.expect("open");
    assert_eq!(step["type"], "form");
    assert_eq!(step["title"], "Login · es");
    assert_eq!(step["buttons"], json!([{"id": "ok", "label": "OK"}]));
    assert_eq!(step["state"]["denied"], "plugin.denied");
    assert_eq!(calls, ["host/item", "host/secret/get"]);
    assert!(stderr.contains("to the log"), "{stderr}");

    let (out, _, stderr) = run(
        "action/submit",
        json!({"action": "greet", "context": ctx, "button": "ok", "values": {"name": "Ñandú"}, "state": {}}),
    );
    let err = out.unwrap_err();
    assert_eq!((err.code, err.detail.as_str()), ("plugin.error.failed", "ValueError: bad Ñandú"));
    assert!(stderr.contains("Traceback"), "{stderr}");

    let conn = json!({"org": "", "project": "board"});
    let (out, _, _) =
        run("tracker/states", json!({"kind": "fake", "connection": conn, "secret": "k3y-value"}));
    assert_eq!(out.unwrap(), json!([{"name": "Doing", "suggested": "active", "project": "board"}]));
    let (out, _, _) =
        run("tracker/connect", json!({"kind": "fake", "connection": conn, "secret": "k3y-value"}));
    assert_eq!(out.unwrap_err().detail, "tracker.auth");
    let (out, _, _) = run("tracker/nope", json!({}));
    assert!(out.unwrap_err().detail.contains("unknown method"));
    let _ = std::fs::remove_dir_all(&config);
}

#[test]
fn the_example_ships_the_current_sdk() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let copy = std::fs::read_to_string(root.join("examples/plugins/hello/workboard_plugin.py")).unwrap();
    assert_eq!(
        copy.replace("\r\n", "\n"),
        cli::PYTHON_SDK.replace("\r\n", "\n"),
        "copy sdk/python/workboard_plugin.py into examples/plugins/hello/"
    );
}

fn opts(parent: &Path, id: &str, lang: Lang, tracker: bool) -> NewOptions {
    NewOptions { id: id.into(), lang, tracker, parent: parent.to_path_buf() }
}

fn check(dir: &Path) -> (bool, String) {
    let mut out = Vec::new();
    let ok = cli::check(dir, &mut out);
    (ok, String::from_utf8(out).unwrap())
}

#[test]
fn new_python_plugin_passes_check() {
    let parent = temp("new-py");
    let dir = cli::create(&opts(&parent, "my-plugin", Lang::Python, true)).unwrap();
    for f in ["manifest.json", "workboard_plugin.py", "plugin.py"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    let m: Json = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(m["trackers"][0]["kind"], "my-plugin");
    assert_eq!(m["actions"][0]["id"], "hello");
    assert_eq!(m["args"], json!(["plugin.py"]));
    // Nunca pisa una carpeta que ya existe.
    let again = cli::create(&opts(&parent, "my-plugin", Lang::Python, false)).unwrap_err();
    assert!(again.contains("already exists"), "{again}");
    let Some(py) = python() else { return };
    let mut m = m;
    m["command"] = json!(py);
    std::fs::write(dir.join("manifest.json"), m.to_string()).unwrap();
    let (ok, out) = check(&dir);
    assert!(ok, "{out}");
    assert!(out.contains("initialize: ok"), "{out}");
    // El handler de ejemplo contesta lo que promete.
    let plugin = wb_plugins::Plugin {
        dir: dir.clone(),
        folder: "my-plugin".into(),
        manifest: Some(
            serde_json::from_value({
                let mut m = m.clone();
                m.as_object_mut().unwrap().remove("trackers");
                m
            })
            .unwrap(),
        ),
        error: None,
        enabled: true,
    };
    let mut host = FakeHost { calls: vec![] };
    let mut stderr = String::new();
    let ctx = json!({"lang": "en", "items": []});
    let step = call(
        &plugin,
        "en",
        "action/open",
        json!({"action": "hello", "context": ctx}),
        &mut host,
        &mut stderr,
    )
    .unwrap();
    assert_eq!(step["type"], "form", "{stderr}");
    let states = call(
        &plugin,
        "en",
        "tracker/states",
        json!({"kind": "my-plugin", "connection": {"org": "", "project": "p"}, "secret": "s"}),
        &mut host,
        &mut stderr,
    )
    .unwrap();
    assert_eq!(states.as_array().map(Vec::len), Some(3));
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn new_node_plugin_passes_check() {
    let parent = temp("new-node");
    let dir = cli::create(&opts(&parent, "nodey", Lang::Node, false)).unwrap();
    for f in ["manifest.json", "workboard-plugin.mjs", "plugin.mjs"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    assert_eq!(std::fs::read_to_string(dir.join("workboard-plugin.mjs")).unwrap(), cli::NODE_SDK);
    let m: Json = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    assert!(m.get("trackers").is_none());
    assert_eq!(m["command"], "node");
    if find(&["node"]).is_none() {
        eprintln!("note: node is not on the PATH, skipping");
        return;
    }
    let (ok, out) = check(&dir);
    assert!(ok, "{out}");
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn check_reports_problems() {
    let parent = temp("check-bad");
    let (ok, out) = check(&parent.join("missing"));
    assert!(!ok);
    assert!(out.contains("problem: "), "{out}");
    let dir = parent.join("bad");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"Bad Id","name":"x","protocol":1,"command":"x"}"#)
        .unwrap();
    let (ok, out) = check(&dir);
    assert!(!ok);
    assert!(out.contains("problem: manifest.json: id"), "{out}");
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"id":"bad","name":"x","protocol":1,"command":"bin/missing","presets":["nope.json"]}"#,
    )
    .unwrap();
    let (ok, out) = check(&dir);
    assert!(!ok);
    assert!(out.contains("problem: preset nope.json"), "{out}");
    assert!(out.contains("problem: command: command not found"), "{out}");
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn cli_arguments() {
    let config = temp("cli");
    let run = |args: &[&str]| {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut out = Vec::new();
        let code = cli::cli(&args, &config, &mut out);
        (code, String::from_utf8(out).unwrap())
    };
    let (code, out) = run(&["new", "from-config", "--lang", "node"]);
    assert_eq!(code, 0, "{out}");
    assert!(config.join("plugins/from-config/plugin.mjs").is_file());
    assert!(out.contains("Settings -> Plugins"), "{out}");
    let (code, out) = run(&["new", "from-config"]);
    assert_eq!(code, 1, "{out}");
    let other = config.join("elsewhere");
    let (code, _) = run(&["new", "dir-one", "--dir", other.to_str().unwrap(), "--tracker"]);
    assert_eq!(code, 0);
    assert!(other.join("dir-one/plugin.py").is_file());
    assert_eq!(run(&["new", "Bad_Id"]).0, 1);
    assert_eq!(run(&["new", "azure", "--tracker"]).0, 1);
    assert_eq!(run(&["new", "x", "--lang", "ruby"]).0, 2);
    assert_eq!(run(&["new"]).0, 2);
    assert_eq!(run(&["check"]).0, 2);
    assert_eq!(run(&["frob"]).0, 2);
    assert_eq!(run(&[]).0, 0);
    let _ = std::fs::remove_dir_all(&config);
}
