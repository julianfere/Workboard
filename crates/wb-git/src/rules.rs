//! Cómo se reconocen los items en nombres de rama y mensajes de commit. Sale del flujo de trabajo (el
//! preset): el código no sabe que en la v1 un item era un número de 6 dígitos.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;

/// Reglas de un flujo de trabajo para el panel Git.
#[derive(Debug, Clone)]
pub struct Rules {
    id: Regex,
    ignore: Option<Regex>,
    /// Prefijo de las ramas que arma la promoción (`promote/`): son copias de commits, no trabajo de un item.
    pub promote_prefix: String,
}

impl Rules {
    /// `id_pattern`: regex con un grupo `id` (lo que rodea al grupo puede consumir un carácter de borde: la
    /// búsqueda sigue desde el final del id, así `a/111111/222222` da los dos). `ignore_pattern`: lo que se
    /// borra de un mensaje antes de buscar ids (el número de un PR no es un item); vacío = nada.
    pub fn new(id_pattern: &str, ignore_pattern: &str, promote_prefix: &str) -> Result<Self, regex::Error> {
        let id = Regex::new(id_pattern)?;
        if !id.capture_names().any(|n| n == Some("id")) {
            return Err(regex::Error::Syntax(format!("{id_pattern}: missing the named group `id`")));
        }
        let ignore = if ignore_pattern.is_empty() { None } else { Some(Regex::new(ignore_pattern)?) };
        Ok(Self { id, ignore, promote_prefix: promote_prefix.to_string() })
    }

    /// Ids que aparecen en un nombre de rama.
    pub fn ids_in_name(&self, text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut pos = 0;
        while pos <= text.len() {
            let Some(c) = self.id.captures_at(text, pos) else { break };
            let Some(m) = c.name("id") else { break };
            out.insert(m.as_str().to_string());
            let next = m.end().max(c.get(0).map_or(0, |w| w.start()) + 1);
            pos = next.max(pos + 1);
            while pos < text.len() && !text.is_char_boundary(pos) {
                pos += 1;
            }
        }
        out
    }

    /// Ids mencionados en un mensaje de commit (sin el número del PR).
    pub fn ids_in_message(&self, text: &str) -> BTreeSet<String> {
        match &self.ignore {
            Some(re) => self.ids_in_name(&re.replace_all(text, " ")),
            None => self.ids_in_name(text),
        }
    }
}

/// Shas que nombra el trailer de un `cherry-pick -x` (`(cherry picked from commit …)`).
pub fn cp_trailer(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\(cherry picked from commit ([0-9a-f]{7,40})\)").expect("regex"));
    re.captures_iter(text).map(|c| c[1].to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_the_pattern() {
        let r = Rules::new(r"(?:^|\D)(?P<id>\d{6})(?:\D|$)", "", "promote/").unwrap();
        let ids = |s: &str| r.ids_in_name(s).into_iter().collect::<Vec<_>>();
        assert_eq!(ids("wi-111111/222222"), ["111111", "222222"]);
        assert_eq!(ids("111111222222"), Vec::<String>::new());
        assert_eq!(ids("ñ111111ñ"), ["111111"]);
        assert!(Rules::new(r"\d+", "", "").is_err());
    }
}
