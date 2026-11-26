//! `workboard plugin new|check`: herramientas para quien escribe un plugin. `new` arma una carpeta lista
//! para habilitar (manifiesto, el SDK de `sdk/` embebido y un handler de ejemplo); `check` valida el
//! manifiesto y corre `initialize` con el runner real. Textos en inglés: es una herramienta de desarrollo,
//! como `docs/plugins.md`.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};

use crate::manifest::{Manifest, PROTOCOL, Plugin};
use crate::rpc::{self, Host};

/// El SDK de Python (`sdk/python/workboard_plugin.py`), tal cual se copia en un plugin nuevo.
pub const PYTHON_SDK: &str = include_str!("../../../sdk/python/workboard_plugin.py");
/// El SDK de Node (`sdk/node/workboard-plugin.mjs`).
pub const NODE_SDK: &str = include_str!("../../../sdk/node/workboard-plugin.mjs");

const USAGE: &str = "\
Usage:
  workboard plugin new <id> [--lang python|node] [--tracker] [--dir <parent>]
      Creates <config>/plugins/<id>/ (or <parent>/<id>/) with manifest.json, the SDK and a starter handler.
  workboard plugin check <folder>
      Validates the folder's manifest.json and runs `initialize` against the plugin.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Python,
    Node,
}

/// Qué crea `plugin new`.
#[derive(Debug, Clone)]
pub struct NewOptions {
    pub id: String,
    pub lang: Lang,
    pub tracker: bool,
    /// Carpeta donde se crea `<id>/` (por defecto `<config>/plugins`).
    pub parent: PathBuf,
}

/// Línea de comando: `args` sin `plugin`. Devuelve el código de salida (0 bien, 1 falló, 2 uso).
pub fn cli(args: &[String], config_dir: &Path, out: &mut dyn Write) -> i32 {
    let (sub, rest) = match args.split_first() {
        Some((s, r)) => (s.as_str(), r),
        None => ("help", &[][..]),
    };
    match sub {
        "new" => match parse_new(rest, config_dir) {
            Ok(opts) => match create(&opts) {
                Ok(dir) => {
                    let _ = writeln!(out, "Created {}", dir.display());
                    let _ = writeln!(out, "{}", next_steps(&opts, &dir));
                    0
                }
                Err(e) => {
                    let _ = writeln!(out, "error: {e}");
                    1
                }
            },
            Err(e) => {
                let _ = writeln!(out, "error: {e}\n\n{USAGE}");
                2
            }
        },
        "check" => match rest {
            [folder] => {
                if check(Path::new(folder), out) {
                    0
                } else {
                    1
                }
            }
            _ => {
                let _ = writeln!(out, "{USAGE}");
                2
            }
        },
        "help" | "--help" | "-h" => {
            let _ = writeln!(out, "{USAGE}");
            0
        }
        other => {
            let _ = writeln!(out, "error: unknown command {other:?}\n\n{USAGE}");
            2
        }
    }
}

fn parse_new(args: &[String], config_dir: &Path) -> Result<NewOptions, String> {
    let mut id = None;
    let mut lang = Lang::Python;
    let mut tracker = false;
    let mut parent = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--lang" => {
                lang = match it.next().map(String::as_str) {
                    Some("python" | "py") => Lang::Python,
                    Some("node" | "js") => Lang::Node,
                    other => return Err(format!("--lang must be python or node, not {other:?}")),
                }
            }
            "--tracker" => tracker = true,
            "--dir" => parent = Some(PathBuf::from(it.next().ok_or("--dir needs a folder")?)),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s if id.is_none() => id = Some(s.to_string()),
            s => return Err(format!("unexpected argument {s:?}")),
        }
    }
    let id = id.ok_or("missing plugin id")?;
    Ok(NewOptions { id, lang, tracker, parent: parent.unwrap_or_else(|| crate::Plugins::dir_in(config_dir)) })
}

/// `Hello-World` a partir de `hello-world`, para los nombres del manifiesto.
fn title(id: &str) -> String {
    id.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// El intérprete para el manifiesto: `python` o, en Windows sin `python` en el PATH, el lanzador `py`.
fn python_command() -> &'static str {
    let path = std::env::var("PATH").unwrap_or_default();
    if cfg!(windows)
        && wb_platform::which("python", &path).is_none()
        && wb_platform::which("py", &path).is_some()
    {
        "py"
    } else {
        "python"
    }
}

/// El manifiesto de un plugin nuevo.
pub fn manifest_for(opts: &NewOptions) -> Json {
    let (command, entry) = match opts.lang {
        Lang::Python => (python_command(), "plugin.py"),
        Lang::Node => ("node", "plugin.mjs"),
    };
    let name = title(&opts.id);
    let mut m = json!({
        "id": opts.id,
        "name": {"en": name, "es": name},
        "description": {"en": "Created with workboard plugin new.", "es": "Creado con workboard plugin new."},
        "version": "0.1.0",
        "protocol": PROTOCOL,
        "command": command,
        "args": [entry],
        "timeoutSeconds": 60,
        "permissions": [],
        "actions": [{
            "id": "hello",
            "label": {"en": "Say hello", "es": "Saludar"},
            "hint": {"en": "Sample action of the plugin", "es": "Acción de ejemplo del plugin"},
            "icon": "sparkle",
            "placements": ["item", "palette"],
        }],
    });
    if opts.tracker {
        m["trackers"] = json!([{
            "kind": opts.id,
            "name": name,
            "org": {"label": {"en": "Organization", "es": "Organización"}, "placeholder": "my-team", "required": false},
            "project": {"label": {"en": "Project", "es": "Proyecto"}, "placeholder": "project id or URL"},
            "secret": {
                "label": {"en": "API token", "es": "Token de la API"},
                "hint": {"en": "Stored in the system credential store.", "es": "Se guarda en el almacén de credenciales del sistema."},
            },
            "writeSecret": false,
            "itemUrl": "https://example.com/{org}/{project}/items/{id}",
        }]);
    }
    m
}

fn python_handler(opts: &NewOptions) -> String {
    let mut s = String::from(
        r#""""Workboard plugin (see docs/plugins.md in Workboard). `workboard_plugin.py` is the SDK: keep it next
to this file."""
from workboard_plugin import Plugin, TrackerError, button, form, result, text

plugin = Plugin()


@plugin.action("hello")
def hello(ctx):
    """First step: a form. `ctx` has lang, root, placement, items and host."""
    names = ", ".join(it["name"] for it in ctx.items) or "(no items)"
    return form("Hello", [text("name", "Your name", required=True)],
                buttons=[button("greet", "Greet", primary=True)], intro="Started on: " + names)


@hello.submit
def hello_submit(ctx, button, values, state):
    """A button of the form was pressed."""
    return result("Hello", "Hello, %s!" % values["name"])
"#,
    );
    if opts.tracker {
        s.push_str(&format!(
            r#"

@plugin.tracker("{kind}")
class Tracker:
    """Answers Workboard's tracker calls. Replace the sample data with calls to your tracker's API;
    `conn` has `org` and `project`, `secret` is the credential (never log it)."""

    def connect(self, conn, secret):
        if not secret:
            raise TrackerError("tracker.auth", "missing credential")
        return {{"org": conn.org, "project": conn.project}}

    def states(self, conn, secret):
        return [
            {{"name": "To do", "color": "", "category": "todo", "types": ["Task"], "suggested": "planned"}},
            {{"name": "Doing", "color": "", "category": "doing", "types": ["Task"], "suggested": "active"}},
            {{"name": "Done", "color": "", "category": "done", "types": ["Task"], "suggested": "done"}},
        ]

    def fields(self, conn, secret):
        return [{{"ref": "due", "name": "Due date", "type": "date", "role": "due"}}]

    def work_items(self, conn, secret, ids, request):
        return {{i: {{"state": "To do", "title": "Item %s" % i, "type": "Task"}} for i in ids}}

    def assigned_to_me(self, conn, secret, due_field, start, end, exclude):
        return []

    def detail(self, conn, secret, id):
        raise TrackerError("tracker.notFound", "no item %s" % id)

    def set_state(self, conn, secret, id, state):
        return {{"state": state, "changed": None}}
"#,
            kind = opts.id
        ));
    }
    s.push_str("\n\nif __name__ == \"__main__\":\n    plugin.run()\n");
    s
}

fn node_handler(opts: &NewOptions) -> String {
    let mut s = String::from(
        r#"// Workboard plugin (see docs/plugins.md in Workboard). `workboard-plugin.mjs` is the SDK: keep it next
// to this file.
import { Plugin, TrackerError, button, form, result, text } from "./workboard-plugin.mjs";

const plugin = new Plugin();

plugin.action("hello", {
  // First step: a form. `ctx` has lang, root, placement, items and host.
  open(ctx) {
    const names = ctx.items.map((it) => it.name).join(", ") || "(no items)";
    return form("Hello", [text("name", "Your name", { required: true })], {
      buttons: [button("greet", "Greet", { primary: true })],
      intro: `Started on: ${names}`,
    });
  },
  // A button of the form was pressed.
  async submit(ctx, button, values, state) {
    return result("Hello", `Hello, ${values.name}!`);
  },
});
"#,
    );
    if opts.tracker {
        s.push_str(&format!(
            r#"
// Answers Workboard's tracker calls. Replace the sample data with calls to your tracker's API; `conn` has
// `org` and `project`, `secret` is the credential (never log it).
plugin.tracker("{kind}", {{
  connect(conn, secret) {{
    if (!secret) throw new TrackerError("tracker.auth", "missing credential");
    return {{ org: conn.org, project: conn.project }};
  }},
  states: () => [
    {{ name: "To do", color: "", category: "todo", types: ["Task"], suggested: "planned" }},
    {{ name: "Doing", color: "", category: "doing", types: ["Task"], suggested: "active" }},
    {{ name: "Done", color: "", category: "done", types: ["Task"], suggested: "done" }},
  ],
  fields: () => [{{ ref: "due", name: "Due date", type: "date", role: "due" }}],
  workItems: async (conn, secret, ids, request) =>
    Object.fromEntries(ids.map((id) => [id, {{ state: "To do", title: `Item ${{id}}`, type: "Task" }}])),
  assignedToMe: async (conn, secret, dueField, start, end, exclude) => [],
  detail(conn, secret, id) {{
    throw new TrackerError("tracker.notFound", `no item ${{id}}`);
  }},
  setState: async (conn, secret, id, state) => ({{ state, changed: null }}),
}});
"#,
            kind = opts.id
        ));
    }
    s.push_str("\nplugin.run();\n");
    s
}

/// El manifiesto sin las claves que esta versión no conoce todavía (`trackers`, si el host no lo valida):
/// `Err` con el problema, `Ok(notas)` si valida.
fn validate(text: &str) -> Result<(Manifest, Vec<String>), String> {
    match Manifest::parse(text) {
        Ok(m) => Ok((m, vec![])),
        Err(e) if e.contains("unknown field `trackers`") => {
            let mut v: Json = serde_json::from_str(text).map_err(|e| e.to_string())?;
            v.as_object_mut().map(|o| o.remove("trackers"));
            let m = Manifest::parse(&v.to_string())?;
            Ok((m, vec!["trackers: not validated by this Workboard version (ignored here)".into()]))
        }
        Err(e) => Err(e),
    }
}

/// Crea la carpeta del plugin. Nunca pisa una que ya existe.
pub fn create(opts: &NewOptions) -> Result<PathBuf, String> {
    let manifest = manifest_for(opts);
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    validate(&text)?;
    if opts.tracker && opts.id == "azure" {
        return Err("tracker kind \"azure\" is built in: choose another id".into());
    }
    let dir = opts.parent.join(&opts.id);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    let files: [(&str, String); 3] = match opts.lang {
        Lang::Python => [
            ("manifest.json", text + "\n"),
            ("workboard_plugin.py", PYTHON_SDK.to_string()),
            ("plugin.py", python_handler(opts)),
        ],
        Lang::Node => [
            ("manifest.json", text + "\n"),
            ("workboard-plugin.mjs", NODE_SDK.to_string()),
            ("plugin.mjs", node_handler(opts)),
        ],
    };
    std::fs::create_dir_all(&opts.parent).map_err(|e| format!("{}: {e}", opts.parent.display()))?;
    std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, body) in files {
        std::fs::write(dir.join(name), body).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(dir)
}

fn next_steps(opts: &NewOptions, dir: &Path) -> String {
    let entry = match opts.lang {
        Lang::Python => "plugin.py",
        Lang::Node => "plugin.mjs",
    };
    let mut s = format!(
        "Next steps:\n  1. Edit {entry} (the SDK file next to it handles the protocol).\n  2. workboard plugin check \"{}\"\n  3. Enable it in Settings -> Plugins.",
        dir.display()
    );
    if opts.tracker {
        s.push_str("\n  4. Connect a work folder to it in Settings -> Tracker.");
    }
    s
}

/// Rechaza todo pedido: `initialize` no debería hacer ninguno.
struct NoHost;

impl Host for NoHost {
    fn request(&mut self, _method: &str, _params: &Json) -> Result<Json, String> {
        Err("plugin.denied".into())
    }
}

/// Valida `folder/manifest.json` (cada problema en una línea), y si valida, lanza el plugin y le manda
/// `initialize`. No llama acciones ni trackers. `true` si todo anduvo.
pub fn check(folder: &Path, out: &mut dyn Write) -> bool {
    let mut ok = true;
    let mut problem = |out: &mut dyn Write, msg: String| {
        ok = false;
        let _ = writeln!(out, "problem: {msg}");
    };
    let path = folder.join("manifest.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            problem(out, format!("{}: {e}", path.display()));
            return false;
        }
    };
    let (manifest, notes) = match validate(&text) {
        Ok(v) => v,
        Err(e) => {
            problem(out, format!("manifest.json: {e}"));
            return false;
        }
    };
    let _ = writeln!(out, "manifest.json: ok ({} {})", manifest.id, manifest.version);
    for n in notes {
        let _ = writeln!(out, "note: {n}");
    }
    for rel in &manifest.presets {
        match std::fs::read_to_string(folder.join(rel)) {
            Ok(t) => {
                if let Err(e) = wb_core::Workflow::from_json(&t) {
                    problem(out, format!("preset {rel}: {e}"));
                }
            }
            Err(e) => problem(out, format!("preset {rel}: {e}")),
        }
    }
    let folder_name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let plugin = Plugin {
        dir: folder.to_path_buf(),
        folder: folder_name,
        manifest: Some(manifest),
        error: None,
        enabled: false,
    };
    if let Err(e) = plugin.argv() {
        problem(out, format!("command: {e}"));
        return false;
    }
    let mut stderr = String::new();
    match rpc::handshake(&plugin, "en", &mut NoHost, &mut stderr) {
        Ok(_) => {
            let _ = writeln!(out, "initialize: ok (protocol {PROTOCOL})");
        }
        Err(e) => {
            let detail = if e.detail.is_empty() { String::new() } else { format!(": {}", e.detail) };
            problem(out, format!("initialize: {}{detail}", e.code));
            if !stderr.trim().is_empty() {
                let _ = writeln!(out, "plugin stderr:\n{}", stderr.trim_end());
            }
        }
    }
    let _ = writeln!(out, "{}", if ok { "OK" } else { "FAILED" });
    ok
}
