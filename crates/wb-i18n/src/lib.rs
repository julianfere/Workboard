//! Traducciones. Los catálogos (`locales/<lang>.json`, claves planas y `{param}`) son los mismos que usa
//! la UI. El inglés es la referencia: una clave que falta en otro idioma sale en inglés, y una que falta
//! en todos sale como la clave misma (se ve el error en vez de un texto vacío).
//!
//! Los errores del backend llevan un código (= clave) y sus parámetros; la UI los traduce. Así el mismo
//! error se puede mostrar en cualquier idioma y comparar en los tests sin depender del texto.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Idiomas incluidos. El primero es la referencia y el respaldo.
pub const LANGS: &[&str] = &["en", "es"];
pub const DEFAULT_LANG: &str = "en";

const SOURCES: &[(&str, &str)] =
    &[("en", include_str!("../../../locales/en.json")), ("es", include_str!("../../../locales/es.json"))];

type Catalog = HashMap<String, String>;

fn catalogs() -> &'static HashMap<&'static str, Catalog> {
    static CATALOGS: OnceLock<HashMap<&'static str, Catalog>> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        SOURCES
            .iter()
            .map(|(lang, json)| {
                (*lang, serde_json::from_str(json).unwrap_or_else(|e| panic!("locales/{lang}.json: {e}")))
            })
            .collect()
    })
}

/// El idioma soportado más cercano a `tag` (`es-AR` → `es`, `fr` → `en`).
pub fn resolve_lang(tag: &str) -> &'static str {
    let base = tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    LANGS.iter().copied().find(|l| *l == base).unwrap_or(DEFAULT_LANG)
}

/// Texto de `key` en `lang` con los `{param}` reemplazados.
pub fn t(lang: &str, key: &str, params: &[(&str, &str)]) -> String {
    let cats = catalogs();
    let text = cats
        .get(resolve_lang(lang))
        .and_then(|c| c.get(key))
        .or_else(|| cats[DEFAULT_LANG].get(key))
        .map_or(key, String::as_str);
    interpolate(text, params)
}

fn interpolate(text: &str, params: &[(&str, &str)]) -> String {
    let mut out = text.to_string();
    for (name, value) in params {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Catálogo completo de un idioma (para mandarlo entero a otro proceso o a un agente).
pub fn catalog(lang: &str) -> &'static HashMap<String, String> {
    &catalogs()[resolve_lang(lang)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn params_of(text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = text;
        while let Some(i) = rest.find('{') {
            let Some(j) = rest[i..].find('}') else { break };
            out.insert(rest[i + 1..i + j].to_string());
            rest = &rest[i + j + 1..];
        }
        out
    }

    #[test]
    fn all_languages_have_the_same_keys_and_params() {
        let reference = catalog(DEFAULT_LANG);
        for lang in LANGS {
            let cat = catalog(lang);
            let missing: Vec<_> = reference.keys().filter(|k| !cat.contains_key(*k)).collect();
            let extra: Vec<_> = cat.keys().filter(|k| !reference.contains_key(*k)).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{lang}: missing {missing:?}, extra {extra:?}");
            for (key, text) in cat {
                assert_eq!(params_of(text), params_of(&reference[key]), "{lang}: params of {key}");
                assert!(!text.trim().is_empty(), "{lang}: empty text for {key}");
            }
        }
    }

    #[test]
    fn lookup_fallbacks_and_params() {
        assert_eq!(t("es", "nav.today", &[]), "Hoy");
        assert_eq!(t("es-AR", "nav.today", &[]), "Hoy");
        assert_eq!(t("fr", "nav.today", &[]), "Today");
        assert_eq!(t("en", "no.such.key", &[]), "no.such.key");
        assert_eq!(
            t("es", "workflow.period", &[("day", "20")]),
            "Los períodos empiezan el día 20 de cada mes"
        );
        assert_eq!(resolve_lang(""), "en");
    }
}
