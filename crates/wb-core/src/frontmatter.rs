//! Frontmatter de los README: un subconjunto de YAML (`k: v` y `k: [a, b]`) con parser y escritor
//! propios. No se usa una librería de YAML a propósito: el formato es el de la v1 y se reescribe igual.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::py;

/// Valor de una clave: texto o lista de textos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Text(String),
    List(Vec<String>),
}

impl Value {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::List(_) => None,
        }
    }

    /// Lo que se considera "sin valor" al escribir (la clave no se escribe).
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(s) => s.is_empty(),
            Self::List(l) => l.is_empty(),
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

/// Claves en el orden del archivo. Pisar una clave conserva su posición; una nueva va al final.
pub type Meta = IndexMap<String, Value>;

/// Separa frontmatter y cuerpo. Sin bloque `---` (o sin cierre) devuelve el texto entero como cuerpo.
pub fn split(text: &str) -> (Meta, String) {
    let lines = py::splitlines(text, true);
    if lines.first().is_none_or(|l| py::strip(l) != "---") {
        return (Meta::new(), text.to_string());
    }
    for i in 1..lines.len() {
        if py::strip(lines[i]) != "---" {
            continue;
        }
        let mut meta = Meta::new();
        for raw in &lines[1..i] {
            if !raw.contains(':') || py::lstrip(raw).starts_with('#') {
                continue;
            }
            let (key, value) = raw.split_once(':').expect("contains ':'");
            meta.insert(py::strip(key).to_string(), parse_value(py::strip(value)));
        }
        let body: String = lines[i + 1..].concat();
        return (meta, body.trim_start_matches('\n').to_string());
    }
    (Meta::new(), text.to_string())
}

fn parse_value(value: &str) -> Value {
    if value.starts_with('[') && value.ends_with(']') && value.len() >= 2 {
        let items = value[1..value.len() - 1]
            .split(',')
            .filter(|v| !py::strip(v).is_empty())
            .map(|v| py::strip(v).trim_matches(['"', '\'']).to_string())
            .collect();
        return Value::List(items);
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() >= 2 && chars[0] == chars[chars.len() - 1] && matches!(chars[0], '"' | '\'') {
        return Value::Text(chars[1..chars.len() - 1].iter().collect());
    }
    Value::Text(value.to_string())
}

fn format_value(value: &Value) -> String {
    match value {
        Value::List(items) => {
            format!("[{}]", items.iter().map(|v| v.replace(',', " ")).collect::<Vec<_>>().join(", "))
        }
        Value::Text(s) => {
            let s = py::strip(&s.replace(['\r', '\n'], " ")).to_string();
            if s.chars().any(|c| ":#[]{}\"'".contains(c)) {
                format!("\"{}\"", s.replace('"', "'"))
            } else {
                s
            }
        }
    }
}

/// Une frontmatter y cuerpo. Las claves vacías no se escriben y sin claves no queda un bloque vacío.
pub fn join(meta: &Meta, body: &str) -> String {
    let pairs: Vec<String> = meta
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}: {}", format_value(v)))
        .collect();
    if pairs.is_empty() {
        return body.to_string();
    }
    let mut out = String::from("---\n");
    for p in pairs {
        out.push_str(&p);
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

/// Decodifica un archivo de texto como la v1: UTF-8 (con o sin BOM), si no cp1252, y si tampoco, UTF-8
/// con reemplazo.
pub fn decode(bytes: &[u8]) -> String {
    let utf8 = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    if let Ok(s) = std::str::from_utf8(utf8) {
        return s.to_string();
    }
    if let Some(s) = decode_cp1252(bytes) {
        return s;
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// cp1252 estricto: los bytes sin carácter asignado (0x81, 0x8D, 0x8F, 0x90, 0x9D) fallan.
fn decode_cp1252(bytes: &[u8]) -> Option<String> {
    const HIGH: [char; 32] = [
        '€', '\0', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\0', 'Ž', '\0', '\0', '‘', '’',
        '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\0', 'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9f => Some(HIGH[usize::from(b - 0x80)]).filter(|c| *c != '\0'),
            _ => Some(char::from(b)),
        })
        .collect()
}

pub fn read(path: &std::path::Path) -> std::io::Result<String> {
    std::fs::read(path).map(|b| decode(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overwrite_keeps_position() {
        let (mut meta, body) = split("---\na: 1\nb: 2\n---\nx\n");
        meta.insert("a".into(), "3".into());
        meta.insert("c".into(), "4".into());
        assert_eq!(join(&meta, &body), "---\na: 3\nb: 2\nc: 4\n---\nx\n");
    }
}
