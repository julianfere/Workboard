//! El MCP contra `tests/golden/cases/mcp.json`: las mismas llamadas de la v1, traducidas a las
//! herramientas nuevas, en los tres modos. Las respuestas se traducen a las claves de la v1.
//!
//! No se compara el texto de `instructions` (la guía se reescribió para el flujo genérico).

use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};
use wb_mcp::{Ctx, ItemUpdate, ToolResult};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn golden() -> Json {
    serde_json::from_str(&std::fs::read_to_string(repo().join("tests/golden/cases/mcp.json")).unwrap())
        .unwrap()
}

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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

const KEYS: &[(&str, &str)] = &[
    ("folder", "carpeta"),
    ("path", "ruta"),
    ("ids", "wis"),
    ("items", "wis"),
    ("title", "titulo"),
    ("status", "estado"),
    ("type", "tipo"),
    ("environments", "ambientes"),
    ("branch", "rama"),
    ("notes", "notas"),
    ("open_tasks", "pendientes_abiertos"),
    ("priority", "prioridad"),
    ("due", "fecha_pactada"),
    ("period", "periodo"),
    ("hours", "horas_pactadas"),
    ("group_of", "grupo_de"),
    ("group", "grupo"),
    ("mode", "modo"),
    ("root", "raiz"),
    ("instructions", "instrucciones"),
    ("tasks", "pendientes"),
    ("index", "indice"),
    ("text", "texto"),
    ("done", "hecha"),
    ("files", "archivos"),
    ("category", "categoria"),
    ("ran", "ejecutado"),
    ("file", "archivo"),
    ("note", "nota"),
];

/// Respuesta nueva → forma de la v1 (el frontmatter queda tal cual).
fn to_v1(v: Json) -> Json {
    match v {
        Json::Array(a) => Json::Array(a.into_iter().map(to_v1).collect()),
        Json::Object(m) => Json::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let new_k = KEYS.iter().find(|(a, _)| *a == k).map_or(k.clone(), |(_, b)| b.to_string());
                    let v = match k.as_str() {
                        "frontmatter" => v,
                        "mode" => json!(match v.as_str().unwrap() {
                            "session" => "sesión",
                            "write" => "escritura",
                            _ => "solo lectura",
                        }),
                        _ => to_v1(v),
                    };
                    (new_k, v)
                })
                .collect(),
        ),
        other => other,
    }
}

/// Rutas → tokens (con `/`, para comparar igual en cualquier SO), hora → `<HH:MM>`, fecha → `<TODAY>`.
fn scrub(v: Json, root: &Path, cfg: &Path) -> Json {
    let today = chrono_today();
    let hhmm = regex::Regex::new(r"<TODAY> \d\d:\d\d").unwrap();
    let mut s = serde_json::to_string(&v).unwrap();
    for (p, token) in [(root, "<ROOT>"), (cfg, "<CONFIG>")] {
        let disp = p.display().to_string();
        for variant in
            [disp.replace('\\', "\\\\"), disp.to_lowercase().replace('\\', "\\\\"), disp.replace('\\', "/")]
        {
            s = s.replace(&variant, token);
        }
    }
    s = s.replace(&today, "<TODAY>");
    s = hhmm.replace_all(&s, "<TODAY> <HH:MM>").into_owned();
    let s = regex::Regex::new(r#"(<ROOT>|<CONFIG>)((?:\\\\[^"\\]*)+)"#)
        .unwrap()
        .replace_all(&s, |c: &regex::Captures| format!("{}{}", &c[1], c[2].replace("\\\\", "/")));
    serde_json::from_str(&s).unwrap()
}

fn chrono_today() -> String {
    // Misma fecha local que usa el MCP para las notas.
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn sort_lists(mut v: Json) -> Json {
    if let Some(o) = v.as_object_mut() {
        if let Some(Json::Array(items)) = o.get_mut("wis")
            && items.first().is_some_and(Json::is_object)
        {
            items.sort_by(|a, b| a["carpeta"].as_str().cmp(&b["carpeta"].as_str()));
        }
        if let Some(Json::Array(files)) = o.get_mut("archivos") {
            files.sort_by(|a, b| a["ruta"].as_str().cmp(&b["ruta"].as_str()));
        }
        o.remove("instrucciones");
    }
    v
}

/// Llamada de la v1 → herramienta nueva.
fn call(ctx: &Ctx, tool: &str, a: &Json) -> ToolResult {
    let st = |k: &str| a.get(k).and_then(Json::as_str).map(String::from);
    let item = st("wi").unwrap_or_default();
    match tool {
        "workboard_session" => wb_mcp::session(ctx),
        "workboard_list_wis" => wb_mcp::list_items(
            ctx,
            &st("buscar").unwrap_or_default(),
            &st("estado").unwrap_or_default(),
            a.get("incluir_cerrados").and_then(Json::as_bool).unwrap_or(false),
        ),
        "workboard_get_wi" => wb_mcp::get_item(ctx, &item),
        "workboard_set_estado" => {
            let upd = ItemUpdate {
                status: st("estado"),
                environments: a
                    .get("ambientes")
                    .and_then(Json::as_array)
                    .map(|l| l.iter().map(|v| v.as_str().unwrap().to_string()).collect()),
                notes: st("notas"),
                branch: st("rama"),
                item_type: st("tipo"),
                due: st("pactada"),
                hours: st("horas"),
            };
            wb_mcp::update_item(ctx, &item, &upd)
        }
        "workboard_add_task" => wb_mcp::add_task(ctx, &item, &st("texto").unwrap()),
        "workboard_complete_task" => wb_mcp::complete_task(
            ctx,
            &item,
            &st("tarea").unwrap(),
            a.get("hecha").and_then(Json::as_bool).unwrap_or(true),
        ),
        "workboard_marcar_ejecutado" => wb_mcp::mark_ran(
            ctx,
            &item,
            &st("archivo").unwrap(),
            &st("ambiente").unwrap(),
            a.get("ejecutado").and_then(Json::as_bool).unwrap_or(true),
        ),
        "workboard_add_note" => wb_mcp::add_note(ctx, &item, &st("titulo").unwrap(), &st("texto").unwrap()),
        "workboard_list_tools" => wb_mcp::list_tools(ctx),
        "workboard_run_tool" => wb_mcp::run_tool(ctx, &st("nombre").unwrap(), &[], &item),
        other => panic!("unknown tool {other}"),
    }
}

fn scenario(name: &str, mode: &str) {
    let g = golden();
    let expected = &g[name];
    let tmp = Temp(std::env::temp_dir().join(format!("wb-mcp-{name}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&tmp.0);
    std::fs::create_dir_all(&tmp.0).unwrap();
    let base = dunce::canonicalize(&tmp.0).unwrap();
    let root = base.join("work");
    copy_dir(&repo().join("tests/golden/fixtures/work"), &root);
    let cfg = base.join("cfg");
    std::fs::create_dir_all(cfg.join("sessions")).unwrap();
    std::fs::write(
        cfg.join("settings.json"),
        json!({"workflow": "classic-es", "lang": "es", "notesFolder": cfg.join("notas")}).to_string(),
    )
    .unwrap();
    let items: serde_json::Map<String, Json> = g["ado_cache_items"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, a)| {
            let mut a = a.clone();
            if let Some(p) = a.as_object_mut().unwrap().remove("pactada") {
                a["due"] = p;
            }
            (id.clone(), a)
        })
        .collect();
    let key = wb_core::records::RunLog::root_key(&root);
    std::fs::write(
        cfg.join(wb_mcp::TRACKER_CACHE),
        json!({ key: {"source": "Azure", "items": items} }).to_string(),
    )
    .unwrap();
    let session = g["session_id"].as_str().unwrap().to_string();
    std::fs::write(
        cfg.join("sessions/work-test.json"),
        json!({"sessions": [{"id": session, "agente": "claude", "wis": ["wi111111-login-con-sso", "grp-checkout"]}]})
            .to_string(),
    )
    .unwrap();
    let ctx = Ctx {
        config_dir: cfg.clone(),
        session: (mode == "session").then_some(session.clone()),
        write_all: mode == "write",
        root: Some(root.clone()),
    };

    let before: Vec<(PathBuf, Vec<u8>)> =
        walk(&root).into_iter().map(|p| (p.clone(), std::fs::read(&p).unwrap())).collect();
    for (i, case) in expected["calls"].as_array().unwrap().iter().enumerate() {
        let tool = case["tool"].as_str().unwrap();
        let ctx_msg = format!("{name} #{i} {tool} {}", case["args"]);
        let result = call(&ctx, tool, &case["args"]);
        let is_v1_bug = tool == "workboard_run_tool"
            && case["message"].as_str().is_some_and(|m| m.contains("'set' object"));
        match (result, case.get("ok")) {
            (Ok(v), Some(ok)) => {
                let ours = sort_lists(scrub(to_v1(v), &root, &cfg));
                let exp = sort_lists(scrub(ok.clone(), &root, &cfg));
                assert_eq!(ours, exp, "{ctx_msg}");
            }
            (Err(e), None) if is_v1_bug => {
                // KNOWN_ISSUES #2: la v1 fallaba; el port usa el primer item de la sesión.
                assert_eq!(e.code, "mcp.tool.notFound", "{ctx_msg}");
            }
            (Err(e), None) => {
                let msg = e.render("es").replace(wb_mcp::names::LIST_ITEMS, "workboard_list_wis");
                assert_eq!(msg, case["message"].as_str().unwrap(), "{ctx_msg}");
            }
            (Ok(v), None) => panic!("{ctx_msg}: expected error {}, got {v}", case["message"]),
            (Err(e), Some(_)) => panic!("{ctx_msg}: unexpected error {}", e.render("es")),
        }
    }

    // Efectos: README que cambiaron, eventos, marcas de ejecutado y notas.
    let changed: serde_json::Map<String, Json> = walk(&root)
        .into_iter()
        .filter(|p| p.file_name().unwrap() == "README.md")
        .filter(|p| {
            before.iter().find(|(b, _)| b == p).is_none_or(|(_, bytes)| *bytes != std::fs::read(p).unwrap())
        })
        .map(|p| {
            let rel = p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            (rel, json!(String::from_utf8_lossy(&std::fs::read(&p).unwrap())))
        })
        .collect();
    assert_eq!(Json::Object(changed), expected["readmes_changed"], "{name}: READMEs");

    let events_file = if mode == "session" {
        cfg.join(format!("sessions/{session}.events.jsonl"))
    } else {
        cfg.join("mcp_events.jsonl")
    };
    let events: Vec<Json> = std::fs::read_to_string(&events_file)
        .unwrap_or_default()
        .lines()
        .map(|l| {
            let mut v: Json = serde_json::from_str(l).unwrap();
            v.as_object_mut().unwrap().remove("ts");
            v
        })
        .collect();
    assert_eq!(Json::Array(events), expected["events"], "{name}: events");

    let runs: Json = std::fs::read_to_string(cfg.join("runs.json")).ok().map_or(json!({}), |s| {
        let raw: Json = serde_json::from_str(&s).unwrap();
        let mapped: serde_json::Map<String, Json> = raw
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, files)| {
                let files: serde_json::Map<String, Json> = files
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(f, envs)| (f.clone(), json!(envs.as_object().unwrap().keys().collect::<Vec<_>>())))
                    .collect();
                (k.clone(), Json::Object(files))
            })
            .collect();
        Json::Object(mapped)
    });
    assert_eq!(scrub(runs, &root, &cfg), expected["runs"], "{name}: runs");

    let notes: serde_json::Map<String, Json> = std::fs::read_dir(cfg.join("notas"))
        .map(|rd| {
            rd.flatten()
                .map(|e| {
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        json!(std::fs::read_to_string(e.path()).unwrap()),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(scrub(Json::Object(notes), &root, &cfg), expected["notes"], "{name}: notes");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push(e.path());
        }
    }
    out
}

#[test]
fn read_only() {
    scenario("read_only", "read");
}

#[test]
fn write_all() {
    scenario("write_all", "write");
}

#[test]
fn session() {
    scenario("session", "session");
}
