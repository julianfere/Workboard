//! Comandos de notas que van más allá de leer, crear y guardar: carpeta efectiva, cambiar el título,
//! vincular items, mostrar en la carpeta y borrar a la papelera del sistema.

use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};

use crate::{Api, ApiResult, Args, to_json};

/// Manda la nota a la papelera del sistema; si no se puede, la mueve a `fallback` (fuera de la carpeta
/// de notas, con la fecha adelante). Nunca borra en duro: la carpeta puede ser un vault de Obsidian.
fn delete_note(
    p: &Path,
    fallback: &Path,
    stamp: &str,
    trash: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<Option<PathBuf>> {
    if trash(p).is_ok() && !p.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(fallback)?;
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let target = fallback.join(format!("{stamp}-{name}"));
    std::fs::rename(p, &target)?;
    Ok(Some(target))
}

impl Api {
    pub(crate) fn notes_command(&self, cmd: &str, a: &Args) -> Option<ApiResult<Json>> {
        let out = match cmd {
            "notes_folder" => self.notes().map(|n| {
                json!({"folder": n.folder().display().to_string(), "isDefault": self.settings().notes_folder.is_none()})
            }),
            "note_set_title" => (|| {
                to_json(self.notes()?.set_title(a.str("id")?, a.str("title")?, a.get("mtime").as_f64())?)
            })(),
            "note_link" => (|| to_json(self.notes()?.link(a.str("id")?, a.str("key")?, a.bool("add"))?))(),
            "note_reveal" => (|| {
                wb_platform::reveal_path(&self.notes()?.resolve(a.str("id")?)?)?;
                Ok(Json::Null)
            })(),
            "note_delete" => self.note_delete(a),
            _ => return None,
        };
        Some(out)
    }

    /// `{trash: true}` si fue a la papelera del sistema; si no, `{trash: false, movedTo}`.
    fn note_delete(&self, a: &Args) -> ApiResult<Json> {
        let id = a.str("id")?;
        let p = self.notes()?.resolve(id)?;
        if !p.is_file() {
            return Err(crate::ApiError::new("core.note.missing", &[("id", id)]));
        }
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let moved =
            delete_note(&p, &self.config_dir.join("notes-deleted"), &stamp, wb_platform::move_to_trash)?;
        Ok(match moved {
            None => json!({"trash": true}),
            Some(target) => json!({"trash": false, "movedTo": target.display().to_string()}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-api-notes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn delete_falls_back_to_a_folder_when_the_trash_fails() {
        let d = temp("del");
        let p = d.join("notes/a.md");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "# A\n").unwrap();
        let moved = delete_note(&p, &d.join("deleted"), "20261120-101010", |_| {
            Err(std::io::Error::other("no trash"))
        })
        .unwrap()
        .unwrap();
        assert_eq!(moved, d.join("deleted/20261120-101010-a.md"));
        assert!(!p.exists() && moved.is_file());

        std::fs::write(&p, "# A\n").unwrap();
        let gone = delete_note(&p, &d.join("deleted"), "x", |q| std::fs::remove_file(q)).unwrap();
        assert!(gone.is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn title_link_and_folder_commands() {
        let d = temp("cmds");
        let api = Api::new(d.join("cfg"));
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es"}})).unwrap();
        let info = api.dispatch("notes_folder", &json!({})).unwrap();
        assert_eq!(info["isDefault"], true);
        assert!(info["folder"].as_str().unwrap().ends_with("notes"));

        let note = api.dispatch("note_create", &json!({"title": "Hallazgo", "items": []})).unwrap();
        let id = note["id"].as_str().unwrap();
        let renamed = api
            .dispatch("note_set_title", &json!({"id": id, "title": "Otro título", "mtime": note["mtime"]}))
            .unwrap();
        assert_eq!((renamed["id"].as_str(), renamed["title"].as_str()), (Some(id), Some("Otro título")));
        let linked = api.dispatch("note_link", &json!({"id": id, "key": "589488", "add": true})).unwrap();
        assert_eq!(linked["items"], json!(["589488"]));
        let err = api.dispatch("note_delete", &json!({"id": "nada.md"})).unwrap_err();
        assert_eq!(err.code, "core.note.missing");
        let err = api.dispatch("note_link", &json!({"id": "../x.md", "key": "1", "add": true})).unwrap_err();
        assert_eq!(err.code, "core.note.outside");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
