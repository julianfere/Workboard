//! `workboard_run_tool` sin `item` corre en la carpeta del primer item de la sesión (el del registro, no
//! el primero alfabético) y deja el evento como la v1: `nombre args → código`.

use std::path::PathBuf;

use serde_json::{Value as Json, json};
use wb_mcp::{Ctx, tools};

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!("wb-mcp-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    dunce::canonicalize(&d).unwrap()
}

#[test]
fn run_tool_uses_the_first_session_item() {
    let d = tmp();
    let root = d.join("work");
    for name in ["wi111111-a", "wi222222-b"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("README.md"), "---\nestado: en-curso\n---\n# x\n").unwrap();
    }
    let cfg = d.join("cfg");
    std::fs::create_dir_all(cfg.join("sessions")).unwrap();
    std::fs::write(cfg.join("settings.json"), json!({"workflow": "classic-es", "lang": "es"}).to_string())
        .unwrap();
    std::fs::write(
        cfg.join("sessions/work.json"),
        json!({"sessions": [{"id": "S1", "agent": "claude", "items": ["wi222222-b", "wi111111-a"]}]})
            .to_string(),
    )
    .unwrap();
    let (file, body) = if cfg!(windows) {
        ("where.bat", "@echo off\r\ncd\r\nexit /b 3\r\n")
    } else {
        ("where.sh", "pwd\nexit 3\n")
    };
    let src = d.join(file);
    std::fs::write(&src, body).unwrap();
    tools::promote(&cfg, &src, "where", "", "", json!({}), false).unwrap();

    let ctx = Ctx { config_dir: cfg.clone(), session: Some("S1".into()), write_all: false, root: Some(root) };
    let out = wb_mcp::run_tool(&ctx, "where", &["x".into(), "y".into()], "").unwrap();
    assert_eq!(out["exitCode"], 3);
    assert!(out["stdout"].as_str().unwrap().contains("wi222222-b"), "{out}");

    let events = std::fs::read_to_string(cfg.join("sessions/S1.events.jsonl")).unwrap();
    let ev: Json = serde_json::from_str(events.lines().next().unwrap()).unwrap();
    assert_eq!(ev["wi"], "wi222222-b");
    assert_eq!(ev["detalle"], "where x y → 3");
    std::fs::remove_dir_all(&d).ok();
}
