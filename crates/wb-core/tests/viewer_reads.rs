//! Lecturas del visor de archivos: límites de tamaño y que nada salga de la raíz.

mod common;

use common::TempDir;
use wb_core::error::ErrorKind;
use wb_core::workspace::{MAX_MEDIA_BYTES, MAX_TEXT_BYTES};
use wb_core::{Workflow, Workspace};

fn setup(tag: &str) -> (TempDir, Workspace) {
    let tmp = TempDir::new(tag);
    let root = tmp.0.join("work");
    std::fs::create_dir_all(root.join("wi1-a")).unwrap();
    std::fs::write(root.join("wi1-a/README.md"), "# A\n").unwrap();
    std::fs::write(tmp.0.join("secret.sql"), "select 1").unwrap();
    std::fs::write(tmp.0.join("secret.png"), [0x89, b'P', b'N', b'G']).unwrap();
    let ws = Workspace::open(&root, Workflow::preset("classic-es").unwrap(), None).unwrap();
    (tmp, ws)
}

fn sized(path: &std::path::Path, len: u64) {
    std::fs::File::create(path).unwrap().set_len(len).unwrap();
}

#[test]
fn reads_text_and_media_inside_the_root() {
    let (tmp, ws) = setup("viewer-ok");
    let dir = tmp.0.join("work/wi1-a");
    std::fs::write(dir.join("q.sql"), "select 1;\n").unwrap();
    std::fs::write(dir.join("p.png"), [1u8, 2, 3]).unwrap();
    assert_eq!(ws.read_file("wi1-a/q.sql").unwrap(), "select 1;\n");
    let m = ws.read_media("wi1-a/p.png").unwrap();
    assert_eq!((m.mime, m.data.as_str()), ("image/png", "AQID"));
}

#[test]
fn size_limits() {
    let (tmp, ws) = setup("viewer-size");
    let dir = tmp.0.join("work/wi1-a");
    sized(&dir.join("edge.txt"), MAX_TEXT_BYTES);
    sized(&dir.join("big.txt"), MAX_TEXT_BYTES + 1);
    sized(&dir.join("edge.pdf"), MAX_MEDIA_BYTES);
    sized(&dir.join("big.pdf"), MAX_MEDIA_BYTES + 1);
    assert!(ws.read_file("wi1-a/edge.txt").is_ok());
    assert_eq!(ws.read_file("wi1-a/big.txt").unwrap_err().code, "core.ws.tooLarge");
    assert!(ws.read_media("wi1-a/edge.pdf").is_ok());
    assert_eq!(ws.read_media("wi1-a/big.pdf").unwrap_err().code, "core.ws.tooLarge");
}

#[test]
fn media_only_for_known_types_and_existing_files() {
    let (tmp, ws) = setup("viewer-types");
    std::fs::write(tmp.0.join("work/wi1-a/x.exe"), b"MZ").unwrap();
    assert_eq!(ws.read_media("wi1-a/x.exe").unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(ws.read_media("wi1-a/missing.png").unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(ws.read_file("wi1-a/missing.sql").unwrap_err().kind, ErrorKind::NotFound);
    // Una carpeta no es un archivo.
    assert_eq!(ws.read_file("wi1-a").unwrap_err().kind, ErrorKind::NotFound);
}

#[test]
fn path_traversal_is_rejected() {
    let (tmp, ws) = setup("viewer-traversal");
    let abs = tmp.0.join("secret.sql").to_string_lossy().into_owned();
    let mut rels = vec!["../secret.sql", "wi1-a/../../secret.sql", abs.as_str()];
    if cfg!(windows) {
        rels.push(r"wi1-a\..\..\secret.sql");
    }
    for rel in rels {
        let e = ws.read_file(rel).unwrap_err();
        assert_eq!((e.kind, e.code), (ErrorKind::Permission, "core.ws.outsideRoot"), "{rel}");
    }
    for rel in ["../secret.png", "wi1-a/../../secret.png"] {
        assert_eq!(ws.read_media(rel).unwrap_err().code, "core.ws.outsideRoot", "{rel}");
    }
}

#[test]
fn links_that_leave_the_root_are_rejected() {
    let (tmp, ws) = setup("viewer-link");
    let link = tmp.0.join("work/wi1-a/link.sql");
    let dir_link = tmp.0.join("work").join("wi1-a").join("outside");
    // Una junction no pide permisos especiales (un symlink sí: modo desarrollador).
    #[cfg(windows)]
    let made = (
        std::os::windows::fs::symlink_file(tmp.0.join("secret.sql"), &link).is_ok(),
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&dir_link)
            .arg(&tmp.0)
            .output()
            .is_ok_and(|o| o.status.success()),
    );
    #[cfg(unix)]
    let made = (
        std::os::unix::fs::symlink(tmp.0.join("secret.sql"), &link).is_ok(),
        std::os::unix::fs::symlink(&tmp.0, &dir_link).is_ok(),
    );
    if made.0 {
        assert_eq!(ws.read_file("wi1-a/link.sql").unwrap_err().code, "core.ws.outsideRoot");
    }
    assert!(made.1 || !cfg!(windows), "no se pudo crear la junction");
    if made.1 {
        assert_eq!(ws.read_file("wi1-a/outside/secret.sql").unwrap_err().code, "core.ws.outsideRoot");
        assert_eq!(ws.read_media("wi1-a/outside/secret.png").unwrap_err().code, "core.ws.outsideRoot");
    }
}
