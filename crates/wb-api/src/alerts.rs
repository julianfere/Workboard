//! Alertas desestimadas (`<config>/alerts_off.json`, por carpeta de trabajos). La UI calcula las alertas
//! (dependen del panel Git y de las sesiones) y guarda cada una que se desestima con una clave estable
//! (`<item>|<tipo>`) y una huella de lo que la disparó: si la huella cambia (otra carpeta duplicada, otro
//! repo desparejo…), la alerta vuelve sola. Lo de items que ya no existen se limpia al guardar.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value as Json;

use crate::{Api, ApiResult, Args, to_json};

/// Clave de la alerta → huella.
pub type Dismissed = BTreeMap<String, String>;
type Store = BTreeMap<String, Dismissed>;

fn store_path(config_dir: &Path) -> PathBuf {
    config_dir.join("alerts_off.json")
}

fn load(path: &Path) -> Store {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(path: &Path, store: &Store) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(store).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, path)
}

/// La carpeta de trabajos como clave del archivo (sin distinguir mayúsculas en Windows).
fn root_key(root: &str) -> String {
    if cfg!(windows) { root.replace('/', "\\").to_lowercase() } else { root.to_string() }
}

/// El item de una clave `<item>|<tipo>`.
fn item_of(key: &str) -> &str {
    key.rsplit_once('|').map_or(key, |(name, _)| name)
}

/// Desestima (`on`) o restaura alertas y saca las de items que ya no están en `names`.
fn apply(map: &mut Dismissed, entries: &[(String, String)], on: bool, names: Option<&BTreeSet<String>>) {
    for (key, fp) in entries {
        if on {
            map.insert(key.clone(), fp.clone());
        } else {
            map.remove(key);
        }
    }
    if let Some(names) = names {
        map.retain(|k, _| names.contains(item_of(k)));
    }
}

impl Api {
    pub(crate) fn alerts_command(&self, cmd: &str, a: &Args) -> Option<ApiResult<Json>> {
        let out = match cmd {
            "alerts_dismissed" => self.alerts_dismissed().and_then(to_json),
            "alerts_dismiss" => self.alerts_dismiss(a),
            _ => return None,
        };
        Some(out)
    }

    fn alerts_root(&self) -> ApiResult<String> {
        Ok(root_key(&self.workspace()?.root().display().to_string()))
    }

    fn alerts_dismissed(&self) -> ApiResult<Dismissed> {
        let root = self.alerts_root()?;
        Ok(load(&store_path(&self.config_dir)).remove(&root).unwrap_or_default())
    }

    /// `entries`: `[{key, fp}]`; `on`: desestimar (true) o restaurar.
    fn alerts_dismiss(&self, a: &Args) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let root = root_key(&ws.root().display().to_string());
        let entries: Vec<(String, String)> = a
            .get("entries")
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let key = e["key"].as_str()?.to_string();
                Some((key, e["fp"].as_str().unwrap_or("").to_string()))
            })
            .collect();
        let names: Option<BTreeSet<String>> =
            ws.list_items().ok().map(|l| l.into_iter().map(|i| i.name).collect());
        let path = store_path(&self.config_dir);
        let mut store = load(&path);
        let map = store.entry(root.clone()).or_default();
        apply(map, &entries, a.bool("on"), names.as_ref());
        let out = map.clone();
        if out.is_empty() {
            store.remove(&root);
        }
        save(&path, &store)?;
        to_json(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dismiss_restore_and_prune() {
        let mut map = Dismissed::new();
        let names: BTreeSet<String> = ["wi1-a", "grp-x/wi2-b"].iter().map(|s| s.to_string()).collect();
        let e = |k: &str, fp: &str| (k.to_string(), fp.to_string());
        apply(
            &mut map,
            &[e("wi1-a|dupes", "wi1-c"), e("grp-x/wi2-b|plan", "plan"), e("gone|noReadme", "")],
            true,
            Some(&names),
        );
        assert_eq!(map.len(), 2, "lo de un item que ya no existe se limpia");
        assert_eq!(map["wi1-a|dupes"], "wi1-c");
        apply(&mut map, &[e("wi1-a|dupes", "")], false, None);
        assert_eq!(map.keys().collect::<Vec<_>>(), ["grp-x/wi2-b|plan"]);
        assert_eq!(item_of("a|b|gitDesync"), "a|b");
    }

    #[test]
    fn dismissed_alerts_are_kept_per_root() {
        use serde_json::json;
        let base = std::env::temp_dir().join(format!("wb-api-alerts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for root in ["one", "two"] {
            std::fs::create_dir_all(base.join(root).join("wi123456-a")).unwrap();
            std::fs::create_dir_all(base.join(root).join("123456-b")).unwrap();
        }
        let api = Api::new(base.join("cfg"));
        api.dispatch("open_root", &json!({"path": base.join("one")})).unwrap();
        assert_eq!(api.dispatch("alerts_dismissed", &json!({})).unwrap(), json!({}));
        let entries =
            json!([{"key": "123456-b|noPrefix", "fp": ""}, {"key": "wi123456-a|dupes", "fp": "123456-b"}]);
        let out = api.dispatch("alerts_dismiss", &json!({"entries": entries, "on": true})).unwrap();
        assert_eq!(out, json!({"123456-b|noPrefix": "", "wi123456-a|dupes": "123456-b"}));

        api.dispatch("open_root", &json!({"path": base.join("two")})).unwrap();
        assert_eq!(api.dispatch("alerts_dismissed", &json!({})).unwrap(), json!({}));
        api.dispatch("open_root", &json!({"path": base.join("one")})).unwrap();
        let back = json!([{"key": "wi123456-a|dupes"}]);
        api.dispatch("alerts_dismiss", &json!({"entries": back, "on": false})).unwrap();
        assert_eq!(api.dispatch("alerts_dismissed", &json!({})).unwrap(), json!({"123456-b|noPrefix": ""}));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
