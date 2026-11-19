//! Ciclo de vida de una sesión con un proceso real: un "comando propio" que imprime y termina.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::json;
use wb_core::{Workflow, Workspace};
use wb_sessions::{AgentSettings, CustomAgent, Env, Live, Options, SessionManager, State};

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

fn setup(tag: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("wb-sess-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/fixtures/work");
    copy(&src, &base.join("work"));
    (base.join("work"), base)
}

/// Un comando que imprime una línea con el id de la sesión y termina.
fn echo_agent() -> CustomAgent {
    if cfg!(windows) {
        CustomAgent { command: "cmd".into(), args: "/c echo hola {session_id}".into(), ..Default::default() }
    } else {
        CustomAgent {
            command: "sh".into(),
            args: "-c \"echo hola {session_id}\"".into(),
            ..Default::default()
        }
    }
}

fn manager(cfg: &Path) -> std::sync::Arc<SessionManager> {
    // Cualquier archivo existente sirve de "binario": este agente no lanza ni el MCP ni el hook.
    let exe = std::env::current_exe().unwrap();
    SessionManager::start(Options {
        config_dir: cfg.to_path_buf(),
        mcp_bin: exe.clone(),
        hook_bin: exe,
        env: Box::new(|| Env {
            workflow: Workflow::preset("classic-es").unwrap(),
            lang: "es".into(),
            agents: AgentSettings { custom: echo_agent(), ..Default::default() },
        }),
    })
    .unwrap()
}

/// Lee la salida hasta que termine, contestando la consulta de posición del cursor con la que arranca
/// ConPTY (sin respuesta no dibuja nada).
fn output_until_exit(mgr: &SessionManager, sid: &str) -> String {
    let (mut offset, mut text, mut answered) = (0u64, String::new(), false);
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let r = mgr.read_since(sid, offset).unwrap();
        offset = r["offset"].as_u64().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap()).unwrap();
        text.push_str(&String::from_utf8_lossy(&bytes));
        if !answered && text.contains("\x1b[6n") {
            mgr.write(sid, "\x1b[1;1R").unwrap();
            answered = true;
        }
        if r["exited"].as_bool().unwrap() {
            return text;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the session did not finish: {text:?}");
}

#[test]
fn custom_agent_lifecycle() {
    let (root, base) = setup("life");
    let cfg = base.join("cfg");
    let mgr = manager(&cfg);
    let ws =
        Workspace::open(&root, Workflow::preset("classic-es").unwrap(), Some(cfg.join("backups"))).unwrap();
    let items = vec!["wi222222-333333-pagos".to_string()];

    let err = mgr.create(&ws, &[], "x", "default", "custom", false, None, None, (80, 24)).unwrap_err();
    assert_eq!(err.code, "session.noItems");
    let err = mgr.create(&ws, &items, "x", "yolo", "custom", false, None, None, (80, 24)).unwrap_err();
    assert_eq!(err.code, "session.badMode");

    let rev = mgr.rev();
    let sess =
        mgr.create(&ws, &items, "  Pagos   ", "default", "custom", false, None, None, (80, 24)).unwrap();
    assert_eq!(sess.name, "Pagos");
    assert!(mgr.rev() > rev);
    let out = output_until_exit(&mgr, &sess.id);
    assert!(out.contains(&format!("hola {}", sess.id)), "{out:?}");

    // Al terminar queda pausada; el README la vincula y el contexto lista sus items.
    let deadline = Instant::now() + Duration::from_secs(5);
    while mgr.sessions(&root)[0].running && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let list = mgr.sessions(&root);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].session.state, State::Paused);
    assert_eq!(list[0].status, Live::Paused);
    let readme = std::fs::read_to_string(root.join("wi222222-333333-pagos/README.md")).unwrap();
    assert!(readme.contains(&format!("sesiones: [{}]", sess.id)), "{readme}");
    let ctx = std::fs::read_to_string(cfg.join("ctx").join(&sess.id).join("AGENTS.md")).unwrap();
    assert!(ctx.contains("## wi222222-333333-pagos (#222222, #333333)"), "{ctx}");
    assert!(ctx.contains("## Cómo trabajar con Workboard"));
    let mcp: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(cfg.join(format!("agents/mcp-{}.json", sess.id))).unwrap(),
    )
    .unwrap();
    assert_eq!(mcp["mcpServers"]["workboard"]["env"]["WORKBOARD_SESSION"], sess.id.as_str());
    let activity = std::fs::read_to_string(mgr.activity_path(&sess.id)).unwrap();
    assert!(activity.contains("\"launch\"") && activity.contains("\"exit\""), "{activity}");

    // Otro item mencionado en un pedido: solo los que existen y no son de la sesión.
    let others = mgr.other_items_in(&sess.id, "mirá el 222222, el 111111 y el build 999999");
    assert_eq!(others.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["111111"]);

    // Renombrar, fijar, cerrar y reabrir.
    assert_eq!(mgr.rename(&root, &sess.id, " Nuevo  nombre ").unwrap(), "Nuevo nombre");
    assert_eq!(mgr.rename(&root, &sess.id, "  ").unwrap_err().code, "session.emptyName");
    mgr.pin(&root, &sess.id, true).unwrap();
    mgr.close(&root, &sess.id).unwrap();
    assert_eq!(mgr.sessions(&root)[0].status, Live::Closed);
    mgr.reopen(&root, &sess.id).unwrap();
    let s = &mgr.sessions(&root)[0].session;
    assert!(s.pinned && s.keep_open && s.state == State::Paused);

    // Sumar y quitar items; mover carpetas actualiza el registro.
    let s =
        mgr.add_items(&ws, &sess.id, &["wi111111-login-con-sso".to_string(), "nope".to_string()]).unwrap();
    assert_eq!(s.items, ["wi222222-333333-pagos", "wi111111-login-con-sso"]);
    let mut map = std::collections::BTreeMap::new();
    map.insert("wi111111-login-con-sso".to_string(), "grp-x/wi111111-login-con-sso".to_string());
    mgr.renamed(&ws, &map);
    assert_eq!(mgr.sessions(&root)[0].session.items[1], "grp-x/wi111111-login-con-sso");
    let s = mgr.remove_item(&ws, &sess.id, "grp-x/wi111111-login-con-sso").unwrap();
    assert_eq!(s.items, items);
    assert_eq!(mgr.remove_item(&ws, &sess.id, &items[0]).unwrap_err().code, "session.onlyItem");
    assert_eq!(mgr.send_prompt(&sess.id, "hola").unwrap_err().code, "session.notRunning");

    // Retomar vuelve a lanzar el comando.
    mgr.resume(&ws, &sess.id, (80, 24)).unwrap();
    output_until_exit(&mgr, &sess.id);
    mgr.shutdown();
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn hooks_update_live_status_and_reply() {
    let (root, base) = setup("hooks");
    let cfg = base.join("cfg");
    let mgr = manager(&cfg);
    let ws =
        Workspace::open(&root, Workflow::preset("classic-es").unwrap(), Some(cfg.join("backups"))).unwrap();
    let items = vec!["wi222222-333333-pagos".to_string()];
    let sess = mgr.create(&ws, &items, "", "default", "custom", false, None, None, (80, 24)).unwrap();
    assert_eq!(sess.name, items[0]);

    // El pedido nombra otro item: la respuesta al CLI lleva la nota y la sesión queda con el aviso.
    let out = mgr.hook("UserPromptSubmit", &sess.id, &json!({"prompt": "y el 111111?"}));
    assert!(out.contains("wi111111-"), "{out}");
    assert!(mgr.hook("Stop", &sess.id, &json!({})).is_empty());
    assert!(mgr.hook("Nope", &sess.id, &json!({})).is_empty());
    assert!(mgr.rev() > 1);

    // Por HTTP, como el hook real: sin token, 403.
    let port = std::fs::read_to_string(cfg.join(format!("agents/mcp-{}.json", sess.id))).map(|_| ()).ok();
    assert!(port.is_some());
    output_until_exit(&mgr, &sess.id);
    mgr.shutdown();
    std::fs::remove_dir_all(&base).ok();
}

/// El receptor de hooks contesta solo con el token.
#[test]
fn hook_server_requires_token() {
    let (_, base) = setup("token");
    let mgr = manager(&base.join("cfg"));
    let (port, token) = mgr.hook_endpoint();
    let post = |token: &str| {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let body = "{}";
        write!(
            s,
            "POST /hook/Stop HTTP/1.1\r\nHost: x\r\nX-Workboard-Token: {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut resp = String::new();
        s.read_to_string(&mut resp).unwrap();
        resp
    };
    assert!(post("wrong").starts_with("HTTP/1.1 403"));
    assert!(post(&token).starts_with("HTTP/1.1 200"));
    std::fs::remove_dir_all(&base).ok();
}

/// Horas, métricas y Reflexionar sobre una sesión real con actividad fabricada.
#[test]
fn time_stats_and_reflect() {
    let (root, base) = setup("stats");
    let cfg = base.join("cfg");
    let mgr = manager(&cfg);
    let ws =
        Workspace::open(&root, Workflow::preset("classic-es").unwrap(), Some(cfg.join("backups"))).unwrap();
    let items = vec!["wi222222-333333-pagos".to_string()];
    let err = mgr.reflect(&ws, &items, "custom", (80, 24)).unwrap_err();
    assert_eq!(err.code, "reflect.noSessions");

    let sess = mgr.create(&ws, &items, "Pagos", "default", "custom", false, None, None, (80, 24)).unwrap();
    output_until_exit(&mgr, &sess.id);
    // Un turno de 120 s que termina pidiendo permiso y otro de 170 s.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    let t0 = now - 600.0;
    let rows = [
        json!({"ts": t0, "ev": "UserPromptSubmit", "items": items}),
        json!({"ts": t0 + 120.0, "ev": "Notification", "items": items, "st": "needsYou"}),
        json!({"ts": t0 + 130.0, "ev": "resume", "items": items}),
        json!({"ts": t0 + 300.0, "ev": "Stop", "items": items}),
    ];
    let mut f = std::fs::OpenOptions::new().append(true).open(mgr.activity_path(&sess.id)).unwrap();
    for r in rows {
        writeln!(f, "{r}").unwrap();
    }
    drop(f);
    ws.write_readme(&items[0], |mut meta, body| {
        meta.insert("estado".into(), wb_core::frontmatter::Value::Text("prod".into()));
        Ok(wb_core::write::Change::Parts(meta, body))
    })
    .unwrap();

    let time = mgr.time_report(&root);
    assert_eq!(time.by_item[&items[0]].sessions[&sess.id], 290);
    assert_eq!(time.sessions[0].source, "log");
    assert_eq!(time.sessions[0].active, 290);

    let stats = mgr.stats_report(&ws);
    let row = stats.iter().find(|r| r.id == sess.id).unwrap();
    assert_eq!((row.prompts, row.asks), (1, 1));
    assert!(row.tokens.is_none());
    let out = &row.outcome[&items[0]];
    assert_eq!(out.end.as_deref(), Some("prod"));
    assert_eq!(out.transitions.len(), 1, "{out:?}");

    let (refl, evidence) = mgr.reflect(&ws, &items, "custom", (80, 24)).unwrap();
    assert_eq!(refl.mode, "plan");
    assert!(!refl.plan);
    let text = std::fs::read_to_string(&evidence).unwrap();
    assert!(text.contains("# Evidencia para reflexionar"), "{text}");
    assert!(text.contains(&format!("### {}", items[0])), "{text}");
    assert!(text.contains(&format!("`{}`", sess.id)), "{text}");
    assert!(text.contains("pedidos de permiso o respuesta: 1"), "{text}");
    output_until_exit(&mgr, &refl.id);
    mgr.shutdown();
    std::fs::remove_dir_all(&base).ok();
}
