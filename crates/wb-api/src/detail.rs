//! Comandos del detalle de un item que no son de un solo paso del core: disolver un grupo (saca sus items a
//! la raíz y manda la carpeta del grupo a eliminados) y abrir la carpeta en VS Code.

use serde_json::{Value as Json, json};

use crate::{Api, ApiError, ApiResult, Args};

impl Api {
    pub(crate) fn detail_command(&self, cmd: &str, a: &Args) -> Option<ApiResult<Json>> {
        let out = match cmd {
            "dissolve_group" => (|| self.dissolve_group(a.str("name")?))(),
            "open_in_vscode" => (|| {
                let ws = self.workspace()?;
                let p = ws.safe_path(a.opt_str("path").unwrap_or(""))?;
                wb_platform::open_in_vscode(&p).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound && p.exists() {
                        ApiError::new("detail.error.noVsCode", &[])
                    } else {
                        e.into()
                    }
                })?;
                Ok(Json::Null)
            })(),
            _ => return None,
        };
        Some(out)
    }

    /// Saca los items del grupo a la raíz (sus marcas, sesiones y notas los siguen) y manda la carpeta del
    /// grupo, con lo que tenga propio, a eliminados. Devuelve `{members: [nombres nuevos], deleted}`.
    fn dissolve_group(&self, name: &str) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let group = ws.build_item(&ws.folder_of(name)?)?;
        if group.kind != wb_core::workspace::ItemKind::Group {
            return Err(ApiError::new("core.move.notGroup", &[("dest", name)]));
        }
        let mut members = Vec::new();
        for m in &group.members {
            let new = self.moved(m, |ws| Ok(ws.move_folder(m, "")?))?;
            members.push(new);
        }
        let deleted = self.moved(&group.name, |ws| Ok(ws.trash(&group.name)?))?;
        Ok(json!({"members": members, "deleted": deleted}))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use crate::Api;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-api-detail-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: PathBuf, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn setup(name: &str) -> (Api, PathBuf, PathBuf) {
        let base = temp(name);
        let root = base.join("work");
        write(root.join("grp-pay/README.md"), "---\nestado: en-curso\n---\n# Pagos\n");
        write(root.join("grp-pay/shared.sql"), "select 1;");
        write(root.join("grp-pay/wi111111-a/README.md"), "---\nestado: plan\n---\n# A\n");
        write(root.join("grp-pay/wi111111-a/FIX_a.sql"), "update x;");
        write(root.join("grp-pay/wi222222-b/README.md"), "# B\n");
        write(root.join("wi333333-c/README.md"), "# C\n");
        let api = Api::new(base.join("cfg"));
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": root})).unwrap();
        (api, root, base)
    }

    #[test]
    fn a_group_detail_carries_the_run_marks_of_its_items() {
        let (api, _root, base) = setup("runs");
        api.dispatch("mark_ran", &json!({"path": "grp-pay/wi111111-a/FIX_a.sql", "env": "dev", "on": true}))
            .unwrap();
        api.dispatch("mark_ran", &json!({"path": "grp-pay/shared.sql", "env": "qa", "on": true})).unwrap();
        let g = api.dispatch("get_item", &json!({"name": "grp-pay"})).unwrap();
        let runs = g["runs"].as_object().unwrap();
        assert!(runs["grp-pay/wi111111-a/FIX_a.sql"]["dev"].is_number());
        assert!(runs["grp-pay/shared.sql"]["qa"].is_number());
        // Un item solo ve las suyas (y no las de una carpeta que empieza igual).
        let a = api.dispatch("get_item", &json!({"name": "grp-pay/wi111111-a"})).unwrap();
        assert_eq!(a["runs"].as_object().unwrap().len(), 1);
        let c = api.dispatch("get_item", &json!({"name": "wi333333-c"})).unwrap();
        assert!(c["runs"].as_object().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn dissolving_a_group_moves_its_items_out_and_trashes_the_folder() {
        let (api, root, base) = setup("dissolve");
        api.dispatch("mark_ran", &json!({"path": "grp-pay/wi111111-a/FIX_a.sql", "env": "dev", "on": true}))
            .unwrap();
        let err = api.dispatch("dissolve_group", &json!({"name": "wi333333-c"})).unwrap_err();
        assert_eq!(err.code, "core.move.notGroup");

        let out = api.dispatch("dissolve_group", &json!({"name": "grp-pay"})).unwrap();
        let mut members: Vec<&str> =
            out["members"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        members.sort();
        assert_eq!(members, ["wi111111-a", "wi222222-b"]);
        assert!(root.join("wi111111-a/FIX_a.sql").is_file());
        assert!(!root.join("grp-pay").exists());
        // Lo propio del grupo no se borra: queda en eliminados, y se puede restaurar.
        let deleted = out["deleted"].as_str().unwrap();
        assert!(root.join(deleted).join("shared.sql").is_file());
        // Las marcas de ejecutado siguieron al item.
        let a = api.dispatch("get_item", &json!({"name": "wi111111-a"})).unwrap();
        assert!(a["runs"]["wi111111-a/FIX_a.sql"]["dev"].is_number());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn opening_a_missing_folder_in_vscode_fails() {
        let (api, _root, base) = setup("code");
        let err = api.dispatch("open_in_vscode", &json!({"path": "nope"})).unwrap_err();
        assert_ne!(err.code, "detail.error.noVsCode");
        let err = api.dispatch("open_in_vscode", &json!({"path": "../outside"})).unwrap_err();
        assert_ne!(err.code, "detail.error.noVsCode");
        let _ = std::fs::remove_dir_all(&base);
    }
}
