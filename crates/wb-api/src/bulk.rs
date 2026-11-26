//! Acciones en lote del menú de la grilla (v1 `bulk_update`, `restore_meta`, `delete_items`,
//! `restore_deleted`, `app.py:818-848, 1476-1502`): cada carpeta por separado (un error no corta el resto
//! y vuelve en `errors`) y la respuesta trae lo necesario para deshacer lo que sí se hizo.

use indexmap::IndexMap;
use serde_json::{Value as Json, json};
use wb_core::frontmatter::Value;
use wb_core::validate::validate_fields;

use crate::{Api, ApiError, ApiResult, Args};

/// Lista de textos de un argumento (lo que no es texto se ignora).
fn strings(v: &Json) -> Vec<String> {
    v.as_array().map_or(vec![], |l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect())
}

impl Api {
    pub(crate) fn bulk_command(&self, cmd: &str, a: &Args) -> Option<ApiResult<Json>> {
        let out = match cmd {
            "bulk_update" => self.bulk_update(
                &strings(a.get("names")),
                a.get("fields"),
                &strings(a.get("addEnvs")),
                &strings(a.get("removeEnvs")),
            ),
            "restore_meta" => self.restore_meta(a.get("entries")),
            "delete_items" => self.delete_items(&strings(a.get("names"))),
            "restore_deleted" => self.restore_deleted(a.get("entries")),
            _ => return None,
        };
        Some(out)
    }

    /// Los mismos cambios de frontmatter en varias carpetas. `fields` se valida una vez (un valor inválido
    /// no escribe nada); `addEnvs`/`removeEnvs` editan la lista de ambientes sin pisar el resto: los del
    /// preset van en su orden y los desconocidos que ya tenía se conservan al final.
    /// Devuelve `{done, errors: [{name, error}], before: [{name, fields}]}` (`before`: valores previos de
    /// las claves tocadas, para `restore_meta`).
    fn bulk_update(
        &self,
        names: &[String],
        fields: &Json,
        add: &[String],
        remove: &[String],
    ) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let wf = ws.workflow();
        let raw: IndexMap<String, Json> = fields
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let fields = validate_fields(wf, &raw)?;
        let known: Vec<&str> = wf.environments.iter().map(|e| e.id.as_str()).collect();
        let add: Vec<&str> = add.iter().map(String::as_str).filter(|e| known.contains(e)).collect();
        let remove: Vec<&str> = remove.iter().map(String::as_str).filter(|e| known.contains(e)).collect();
        let env_key = &wf.fields.environments;
        let touches_envs = !add.is_empty() || !remove.is_empty();
        let mut touched: Vec<&String> = fields.keys().collect();
        if touches_envs && !touched.contains(&env_key) {
            touched.push(env_key);
        }
        let (mut done, mut errors, mut before) = (vec![], vec![], vec![]);
        for name in names {
            let res = ws.get_meta(name).and_then(|meta| {
                let mut change = fields.clone();
                if touches_envs {
                    let mut envs: Vec<String> = meta
                        .get(env_key)
                        .map(Value::as_list)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|e| !remove.contains(&e.as_str()))
                        .collect();
                    for e in &add {
                        if !envs.iter().any(|x| x == e) {
                            envs.push((*e).to_string());
                        }
                    }
                    let mut list: Vec<String> = known
                        .iter()
                        .filter(|k| envs.iter().any(|e| e == *k))
                        .map(|k| (*k).to_string())
                        .collect();
                    list.extend(envs.iter().filter(|e| !known.contains(&e.as_str())).cloned());
                    change.insert(env_key.clone(), Value::List(list));
                }
                let prev: IndexMap<&String, Value> = touched
                    .iter()
                    .map(|k| (*k, meta.get(*k).cloned().unwrap_or_else(|| Value::Text(String::new()))))
                    .collect();
                ws.update_meta(name, &change)?;
                Ok(prev)
            });
            match res {
                Ok(prev) => {
                    before.push(json!({"name": name, "fields": prev}));
                    done.push(name.clone());
                }
                Err(e) => errors.push(json!({"name": name, "error": ApiError::from(e)})),
            }
        }
        Ok(json!({"done": done, "errors": errors, "before": before}))
    }

    /// Deshace `bulk_update`: vuelve a escribir los valores previos tal cual, sin validarlos (un valor que el
    /// flujo ya no acepta igual estaba en el archivo). `entries`: `[{name, fields}]`.
    fn restore_meta(&self, entries: &Json) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let (mut done, mut errors) = (vec![], vec![]);
        for e in entries.as_array().into_iter().flatten() {
            let name = e["name"].as_str().unwrap_or("");
            let res = serde_json::from_value::<IndexMap<String, Value>>(e["fields"].clone())
                .map_err(|_| ApiError::new("error.badArgument", &[("name", "fields")]))
                .and_then(|fields| Ok(ws.update_meta(name, &fields)?));
            match res {
                Ok(_) => done.push(name.to_string()),
                Err(err) => errors.push(json!({"name": name, "error": err})),
            }
        }
        Ok(json!({"done": done, "errors": errors}))
    }

    /// Manda carpetas a eliminados. Un item de un grupo que también se elimina no se mueve aparte (va con su
    /// grupo). Devuelve `{done: [{name, to}], errors: [{name, error}]}`; `done` sirve para `restore_deleted`.
    fn delete_items(&self, names: &[String]) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let wf = ws.workflow();
        let groups: Vec<&str> =
            names.iter().map(String::as_str).filter(|n| !n.contains('/') && wf.is_group_folder(n)).collect();
        // Primero los items y después los grupos, como la v1.
        let mut order: Vec<&String> = names.iter().collect();
        order.sort_by_key(|n| groups.contains(&n.as_str()));
        let (mut done, mut errors) = (vec![], vec![]);
        for name in order {
            if name.split_once('/').is_some_and(|(g, _)| groups.contains(&g)) {
                continue;
            }
            match self.moved(name, |ws| Ok(ws.trash(name)?)) {
                Ok(to) => done.push(json!({"name": name, "to": to})),
                Err(e) => errors.push(json!({"name": name, "error": e})),
            }
        }
        Ok(json!({"done": done, "errors": errors}))
    }

    /// Vuelve carpetas de eliminados: `entries` = `[{to, name}]` (lo que devolvió `delete_items`, o una
    /// entrada de la lista de eliminados con su nombre original). Devuelve `{done: [nombres], errors}`.
    fn restore_deleted(&self, entries: &Json) -> ApiResult<Json> {
        let (mut done, mut errors) = (vec![], vec![]);
        for e in entries.as_array().into_iter().flatten() {
            let (to, name) = (e["to"].as_str().unwrap_or(""), e["name"].as_str().unwrap_or(""));
            match self.moved(to, |ws| Ok(ws.untrash(to, name)?)) {
                Ok(n) => done.push(n.as_str().map(String::from).unwrap_or_default()),
                Err(err) => errors.push(json!({"name": name, "error": err})),
            }
        }
        Ok(json!({"done": done, "errors": errors}))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value as Json, json};

    use crate::Api;

    fn write(p: PathBuf, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn setup(name: &str) -> (Api, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("wb-api-bulk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("work");
        write(root.join("grp-pay/README.md"), "---\nestado: en-curso\n---\n# Pagos\n");
        write(root.join("grp-pay/wi111111-a/README.md"), "---\nestado: plan\n---\n# A\n");
        write(root.join("grp-pay/wi222222-b/README.md"), "# B\n");
        write(
            root.join("wi333333-c/README.md"),
            "---\nestado: pendiente\nambientes: [prod, zz, dev]\n---\n# C\n",
        );
        write(root.join("wi444444-d/README.md"), "---\nestado: rarísimo\n---\n# D\n");
        std::fs::create_dir_all(root.join("wi555555-e")).unwrap();
        let api = Api::new(base.join("cfg"));
        api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es"}})).unwrap();
        api.dispatch("open_root", &json!({"path": root})).unwrap();
        (api, root, base)
    }

    fn meta(api: &Api, name: &str) -> Json {
        api.dispatch("get_item", &json!({"name": name})).unwrap()["item"]["meta"].clone()
    }

    #[test]
    fn bulk_envs_keep_unknown_ones_and_report_per_folder() {
        let (api, _root, base) = setup("envs");
        let out = api
            .dispatch(
                "bulk_update",
                &json!({"names": ["wi333333-c", "nope", "wi444444-d"], "fields": {}, "addEnvs": ["qa", "xx"], "removeEnvs": ["prod"]}),
            )
            .unwrap();
        assert_eq!(out["done"], json!(["wi333333-c", "wi444444-d"]));
        assert_eq!(out["errors"][0]["name"], "nope");
        assert!(out["errors"][0]["error"]["code"].is_string());
        // Los del preset en su orden; el desconocido se conserva al final.
        assert_eq!(meta(&api, "wi333333-c")["ambientes"], json!(["dev", "qa", "zz"]));
        assert_eq!(meta(&api, "wi444444-d")["ambientes"], json!(["qa"]));
        assert_eq!(
            out["before"][0],
            json!({"name": "wi333333-c", "fields": {"ambientes": ["prod", "zz", "dev"]}})
        );
        assert_eq!(out["before"][1], json!({"name": "wi444444-d", "fields": {"ambientes": ""}}));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn bulk_update_validates_once_and_undo_restores_without_validating() {
        let (api, _root, base) = setup("undo");
        let err = api
            .dispatch("bulk_update", &json!({"names": ["wi333333-c"], "fields": {"estado": "x"}}))
            .unwrap_err();
        assert_eq!(err.code, "core.field.oneOf");
        let out = api
            .dispatch(
                "bulk_update",
                &json!({"names": ["wi333333-c", "wi444444-d"], "fields": {"estado": "qa"}}),
            )
            .unwrap();
        assert_eq!(meta(&api, "wi444444-d")["estado"], "qa");
        // Deshacer vuelve el estado que el flujo no conoce (sin validar).
        let back = api.dispatch("restore_meta", &json!({"entries": out["before"]})).unwrap();
        assert_eq!(back["done"], json!(["wi333333-c", "wi444444-d"]));
        assert_eq!(meta(&api, "wi444444-d")["estado"], "rarísimo");
        assert_eq!(meta(&api, "wi333333-c")["estado"], "pendiente");
        // «Sin estado»: vacío borra la clave, y deshacer la vuelve.
        let out =
            api.dispatch("bulk_update", &json!({"names": ["wi333333-c"], "fields": {"estado": ""}})).unwrap();
        assert!(meta(&api, "wi333333-c").get("estado").is_none());
        api.dispatch("restore_meta", &json!({"entries": out["before"]})).unwrap();
        assert_eq!(meta(&api, "wi333333-c")["estado"], "pendiente");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn bulk_update_with_no_fields_creates_a_missing_readme() {
        let (api, root, base) = setup("readme");
        let out = api.dispatch("bulk_update", &json!({"names": ["wi555555-e"], "fields": {}})).unwrap();
        assert_eq!(out["done"], json!(["wi555555-e"]));
        assert!(root.join("wi555555-e/README.md").is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn delete_skips_members_of_a_deleted_group_and_undo_restores() {
        let (api, root, base) = setup("delete");
        let out = api
            .dispatch(
                "delete_items",
                &json!({"names": ["grp-pay", "grp-pay/wi111111-a", "wi333333-c", "nope"]}),
            )
            .unwrap();
        let done = out["done"].as_array().unwrap();
        assert_eq!(done.len(), 2);
        assert_eq!(done[0]["name"], "wi333333-c");
        assert_eq!(done[1]["name"], "grp-pay");
        assert_eq!(out["errors"][0]["name"], "nope");
        assert!(root.join("_deleted/grp-pay/wi111111-a/README.md").is_file());
        assert!(!root.join("wi333333-c").exists());

        let back = api.dispatch("restore_deleted", &json!({"entries": out["done"]})).unwrap();
        assert_eq!(back["done"], json!(["wi333333-c", "grp-pay"]));
        assert!(root.join("grp-pay/wi111111-a/README.md").is_file());
        assert!(root.join("wi333333-c/README.md").is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn deleted_entries_restore_with_their_original_name() {
        let (api, root, base) = setup("original");
        let first = api.dispatch("delete_items", &json!({"names": ["wi333333-c"]})).unwrap();
        write(root.join("wi333333-c/README.md"), "# C otra\n");
        api.dispatch("delete_items", &json!({"names": ["wi333333-c"]})).unwrap();
        let list = api.dispatch("list_deleted", &json!({})).unwrap();
        let stamped = list.as_array().unwrap().iter().find(|d| d["base"] != "wi333333-c").unwrap();
        assert_eq!(stamped["original"], "wi333333-c");
        let back = api
            .dispatch(
                "restore_deleted",
                &json!({"entries": [{"to": stamped["path"], "name": stamped["original"]}]}),
            )
            .unwrap();
        assert_eq!(back["done"], json!(["wi333333-c"]));
        // El otro ya no entra con el mismo nombre: error por carpeta, sin cortar.
        let again = api.dispatch("restore_deleted", &json!({"entries": first["done"]})).unwrap();
        assert_eq!(again["errors"][0]["error"]["code"], "core.trash.exists");
        let _ = std::fs::remove_dir_all(&base);
    }
}
