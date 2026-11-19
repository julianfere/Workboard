//! Contexto que recibe un agente al arrancar una sesión: sus items (carpeta, estado, resumen, tareas
//! abiertas), la guía de Workboard y las tools disponibles. La app lo escribe en
//! `<config>/ctx/<id>/AGENTS.md` (cada CLI lo carga a su manera) y el MCP lo repite en sus `instructions`,
//! así cualquier cliente arranca sabiendo cómo trabajar aunque no lea el archivo.

use std::path::Path;

use serde_json::Value as Json;
use wb_core::Workflow;
use wb_core::frontmatter::Value;
use wb_core::workspace::{Item, ItemKind};

use crate::{item_path, names};

/// Guía de Workboard para un flujo e idioma (las `instructions` del MCP).
pub fn guide_for(wf: &Workflow, lang: &str) -> String {
    let ids = |v: Vec<&str>| v.join(", ");
    wb_i18n::t(
        lang,
        "mcp.guide",
        &[
            ("states", &ids(wf.state_ids().collect())),
            ("types", &ids(wf.types.iter().map(|t| t.id.as_str()).collect())),
            ("environments", &ids(wf.environments.iter().map(|e| e.id.as_str()).collect())),
            ("tasks", &wf.tasks.heading),
            ("session", names::SESSION),
            ("get", names::GET_ITEM),
            ("update", names::UPDATE_ITEM),
            ("addTask", names::ADD_TASK),
            ("completeTask", names::COMPLETE_TASK),
            ("addNote", names::ADD_NOTE),
            ("markRan", names::MARK_RAN),
            ("listTools", names::LIST_TOOLS),
            ("runTool", names::RUN_TOOL),
        ],
    )
}

fn text<'a>(it: &'a Item, key: &str) -> &'a str {
    it.meta.get(key).and_then(Value::as_text).unwrap_or("")
}

fn dash(s: &str) -> &str {
    if s.is_empty() { "—" } else { s }
}

/// Líneas de un item (y, si es un grupo, de sus items un nivel más abajo).
pub fn item_lines(
    wf: &Workflow,
    lang: &str,
    root: &Path,
    name: &str,
    all: &[Item],
    level: &str,
) -> Vec<String> {
    let t = |k: &str, p: &[(&str, &str)]| wb_i18n::t(lang, k, p);
    let Some(it) = all.iter().find(|it| it.name == name) else {
        return vec![format!("{level} {name}"), format!("- {}", t("ctx.missing", &[])), String::new()];
    };
    let folder = item_path(root, name);
    let keys = &wf.fields;
    let mut lines = if it.kind == ItemKind::Group {
        vec![
            format!(
                "{level} {}",
                t("ctx.group", &[("name", name), ("count", &it.members.len().to_string())])
            ),
            format!("- {}", t("ctx.groupFolder", &[("path", &folder.display().to_string())])),
        ]
    } else {
        let ids = if it.ids.is_empty() { String::new() } else { format!(" (#{})", it.ids.join(", #")) };
        let envs = it.meta.get(&keys.environments).map(|v| v.as_list().join(", ")).unwrap_or_default();
        let mut l = vec![format!("{level} {name}{ids}")];
        if !it.title.is_empty() {
            l.push(format!("- {}", t("ctx.title", &[("title", &it.title)])));
        }
        l.push(format!("- {}", t("ctx.folder", &[("path", &folder.display().to_string())])));
        l.push(format!(
            "- {}",
            t(
                "ctx.state",
                &[
                    ("status", dash(text(it, &keys.status))),
                    ("environments", dash(&envs)),
                    ("branch", dash(text(it, &keys.branch))),
                ],
            )
        ));
        l
    };
    let notes = text(it, &keys.notes);
    if !notes.is_empty() {
        lines.push(format!("- {}", t("ctx.summary", &[("notes", notes)])));
    }
    let open: Vec<&str> = it.tasks.iter().filter(|x| !x.done).map(|x| x.text.as_str()).collect();
    if !open.is_empty() {
        lines.push(format!("- {}", t("ctx.openTasks", &[])));
        lines.extend(open.iter().map(|x| format!("  - [ ] {x}")));
    }
    lines.push(String::new());
    for member in &it.members {
        lines.extend(item_lines(wf, lang, root, member, all, &format!("{level}#")));
    }
    lines
}

/// Las tools promovidas, para el contexto (vacío si no hay).
fn tools_lines(lang: &str, tools: &[Json]) -> Vec<String> {
    if tools.is_empty() {
        return vec![];
    }
    let mut lines = vec![wb_i18n::t(lang, "ctx.tools", &[("tool", names::RUN_TOOL)]), String::new()];
    for tool in tools {
        let field = |k: &str| tool[k].as_str().unwrap_or("").to_string();
        let usage = field("uso");
        let desc = field("descripcion");
        let mut line = format!("- `{}`", field("nombre"));
        if !usage.is_empty() {
            line.push_str(&format!(" {usage}"));
        }
        if !desc.is_empty() {
            line.push_str(&format!(": {desc}"));
        }
        lines.push(line);
    }
    lines.push(String::new());
    lines
}

/// Contexto completo de una sesión: encabezado, sus items, la guía y las tools.
pub fn session_context(
    wf: &Workflow,
    lang: &str,
    root: &Path,
    session_name: &str,
    session_items: &[String],
    all: &[Item],
    tools: &[Json],
) -> String {
    let mut lines = vec![
        format!("# {}", wb_i18n::t(lang, "ctx.header", &[("name", session_name)])),
        String::new(),
        wb_i18n::t(lang, "ctx.intro", &[("root", &root.display().to_string())]),
        String::new(),
    ];
    for name in session_items {
        lines.extend(item_lines(wf, lang, root, name, all, "##"));
    }
    lines.push(guide_for(wf, lang));
    lines.push(String::new());
    lines.extend(tools_lines(lang, tools));
    lines.join("\n")
}
