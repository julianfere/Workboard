//! Contexto de una sesión y guía del agente.

use std::path::{Path, PathBuf};

use serde_json::json;
use wb_core::{Workflow, Workspace};
use wb_mcp::{Ctx, context, tools};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/fixtures/work")
}

#[test]
fn group_readme_has_its_own_line() {
    let wf = Workflow::preset("classic-es").unwrap();
    let ws = Workspace::open(&fixture(), wf.clone(), None).unwrap();
    let all = ws.list_items().unwrap();
    let text = context::session_context(&wf, "es", ws.root(), "S", &["grp-checkout".to_string()], &all, &[]);
    let folder = ws.root().join("grp-checkout");
    assert!(
        text.contains(&format!("- Carpeta del grupo (archivos compartidos): {}", folder.display())),
        "{text}"
    );
    assert!(text.contains(&format!("- README del grupo: {}", folder.join("README.md").display())), "{text}");
}

#[test]
fn guide_explains_types_get_item_files_and_example() {
    let wf = Workflow::preset("classic-es").unwrap();
    let g = context::guide_for(&wf, "es");
    assert!(g.contains("us = User story"), "{g}");
    assert!(g.contains("fix = Arreglo (sin WI)"), "{g}");
    assert!(g.contains("prioridad, fecha pactada, período (del 20 al 19) y horas pactadas"), "{g}");
    assert!(g.contains("la ruta está en el contexto de la sesión"), "{g}");
    assert!(g.contains("Con varios trabajos sin grupo"), "{g}");
    assert!(g.contains("`DIAG_WI<número>_<qué mira>.sql`"), "{g}");
    assert!(g.contains("MIGRATION_ / SEED_ / ROLLBACK_ (Migraciones)"), "{g}");
    let en = context::guide_for(&Workflow::preset("default").unwrap(), "en");
    assert!(en.contains("(calendar month)"), "{en}");
    assert!(en.contains("`DIAG_<number>_<what it checks>.sql`"), "{en}");
}

#[test]
fn only_valid_tools_reach_the_context() {
    let d = std::env::temp_dir().join(format!("wb-mcp-ctx-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let cfg = d.join("cfg");
    for name in ["good", "changed"] {
        let src = d.join(format!("{name}.py"));
        std::fs::write(&src, "print(1)\n").unwrap();
        tools::promote(&cfg, &src, name, "desc", "", json!({}), false).unwrap();
    }
    std::fs::write(cfg.join("tools/changed/changed.py"), "print(2)\n").unwrap();
    let ctx = Ctx { config_dir: cfg, session: None, write_all: false, root: None };
    let listed = wb_mcp::registered_tools(&ctx);
    let names: Vec<&str> = listed.iter().filter_map(|t| t["nombre"].as_str()).collect();
    assert_eq!(names, ["good"]);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn instructions_start_with_the_intro() {
    let d = std::env::temp_dir().join(format!("wb-mcp-intro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("settings.json"), json!({"workflow": "classic-es", "lang": "en"}).to_string())
        .unwrap();
    let ctx = Ctx { config_dir: d.clone(), session: None, write_all: false, root: None };
    let text = wb_mcp::instructions(&ctx);
    assert!(text.starts_with("Status of the work items in the work folder"), "{text}");
    assert!(text.contains("`grp-…` group") && text.contains("## How to work with Workboard"), "{text}");
    std::fs::remove_dir_all(&d).ok();
}
