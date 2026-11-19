//! Validación de los campos editables del frontmatter (la usan la UI y el MCP). Valida en vez de
//! corregir: el formato plano del frontmatter aplanaría saltos de línea y comas sin avisar.

use indexmap::IndexMap;
use serde_json::Value as Json;

use crate::error::{CoreError, Result};
use crate::frontmatter::Value;
use crate::{Workflow, py};

pub const MAX_HOURS: f64 = 10000.0;

/// Valida `fields` (clave → valor tal como llega de la UI o del MCP) y devuelve los valores normalizados.
/// `null` se ignora; `""` o `[]` = borrar el campo (se devuelve `""`).
pub fn validate_fields(wf: &Workflow, fields: &IndexMap<String, Json>) -> Result<IndexMap<String, Value>> {
    let keys = &wf.fields;
    let editable = keys.editable();
    let max_len = wf.tasks.max_len;
    let mut out = IndexMap::new();
    for (key, value) in fields {
        if value.is_null() {
            continue;
        }
        if !editable.contains(&key.as_str()) {
            return Err(CoreError::invalid("core.field.notEditable", vec![("key", key.clone())]));
        }
        if is_empty(value) {
            out.insert(key.clone(), Value::Text(String::new()));
            continue;
        }
        let k = key.as_str();
        let normalized = if k == keys.status {
            one_of(k, value, wf.state_ids())?
        } else if k == keys.kind {
            one_of(k, value, wf.types.iter().map(|t| t.id.as_str()))?
        } else if k == keys.environments {
            environments(wf, k, value)?
        } else if k == keys.item_ids {
            item_ids(wf, k, value)?
        } else if k == keys.due {
            due_date(k, value)?
        } else if k == keys.period {
            period(wf, k, value)?
        } else if k == keys.hours {
            let h = parse_hours(&py_str(value))
                .ok_or_else(|| invalid_value("core.field.hours", k, value, ("max", fmt_g(MAX_HOURS))))?;
            Value::Text(fmt_g(h))
        } else {
            let no_spaces = k == keys.branch || keys.extra.iter().any(|f| f.key == k && f.no_spaces);
            text_line(k, value, no_spaces, max_len)?
        };
        out.insert(key.clone(), normalized);
    }
    Ok(out)
}

fn is_empty(v: &Json) -> bool {
    matches!(v, Json::String(s) if s.is_empty()) || matches!(v, Json::Array(a) if a.is_empty())
}

/// `str(value)` de Python para lo que puede llegar en un campo.
fn py_str(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        Json::Bool(b) => (if *b { "True" } else { "False" }).into(),
        Json::Number(n) => {
            n.as_i64().map_or_else(|| n.as_f64().map(py_float_str).unwrap_or_default(), |i| i.to_string())
        }
        Json::Null => "None".into(),
        other => other.to_string(),
    }
}

/// `repr(value)` de Python (texto entre comillas; números y listas como los escribe Python).
fn py_repr(v: &Json) -> String {
    match v {
        Json::String(s) => py::repr(s),
        Json::Array(items) => format!("[{}]", items.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        other => py_str(other),
    }
}

fn py_float_str(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e16 { format!("{f:.1}") } else { f.to_string() }
}

fn invalid_value(code: &'static str, key: &str, value: &Json, extra: (&'static str, String)) -> CoreError {
    CoreError::invalid(code, vec![("key", key.to_string()), ("value", py_repr(value)), extra])
}

fn one_of<'a>(key: &str, value: &Json, valid: impl Iterator<Item = &'a str>) -> Result<Value> {
    let valid: Vec<&str> = valid.collect();
    match value.as_str() {
        Some(s) if valid.contains(&s) => Ok(Value::Text(s.to_string())),
        _ => Err(invalid_value("core.field.oneOf", key, value, ("valid", valid.join(", ")))),
    }
}

fn environments(wf: &Workflow, key: &str, value: &Json) -> Result<Value> {
    let ids: Vec<&str> = wf.environments.iter().map(|e| e.id.as_str()).collect();
    let err = || {
        CoreError::invalid("core.field.environments", vec![("key", key.into()), ("valid", ids.join(", "))])
    };
    let items = value.as_array().ok_or_else(err)?;
    let given: Vec<&str> = items
        .iter()
        .map(|e| e.as_str().filter(|s| ids.contains(s)).ok_or_else(err))
        .collect::<Result<_>>()?;
    Ok(Value::List(ids.iter().filter(|e| given.contains(e)).map(|e| e.to_string()).collect()))
}

fn item_ids(wf: &Workflow, key: &str, value: &Json) -> Result<Value> {
    let raw: Vec<String> = match value {
        Json::Array(items) => items.iter().map(py_str).collect(),
        other => py_str(other).split(|c: char| py::is_space(c) || c == ',').map(String::from).collect(),
    };
    let ids: Vec<String> = raw
        .iter()
        .filter(|w| !py::strip(w).is_empty())
        .map(|w| py::strip(w).trim_start_matches('#').to_string())
        .collect();
    if ids.is_empty() || ids.iter().any(|w| !wf.is_valid_item_id(w)) {
        return Err(CoreError::invalid("core.field.itemIds", vec![("key", key.into())]));
    }
    Ok(if ids.len() == 1 { Value::Text(ids[0].clone()) } else { Value::List(ids) })
}

fn due_date(key: &str, value: &Json) -> Result<Value> {
    let s: String = py::strip(&py_str(value)).chars().take(10).collect();
    if parse_date(&s).is_some_and(|(y, _, _)| (1990..=2100).contains(&y)) {
        Ok(Value::Text(s))
    } else {
        Err(CoreError::invalid("core.field.date", vec![("key", key.into()), ("value", py::repr(&s))]))
    }
}

fn period(wf: &Workflow, key: &str, value: &Json) -> Result<Value> {
    let s = py::strip(&py_str(value)).to_string();
    let ok = s.len() == 7
        && s.as_bytes()[4] == b'-'
        && s[..4].bytes().all(|b| b.is_ascii_digit())
        && s[5..].bytes().all(|b| b.is_ascii_digit())
        && (1..=12).contains(&s[5..].parse::<u32>().unwrap_or(0));
    if ok {
        Ok(Value::Text(s))
    } else {
        Err(CoreError::invalid(
            "core.field.period",
            vec![("key", key.into()), ("value", py::repr(&s)), ("day", wf.period.start_day.to_string())],
        ))
    }
}

fn text_line(key: &str, value: &Json, no_spaces: bool, max_len: usize) -> Result<Value> {
    let s = py::strip(&py_str(value)).to_string();
    let err = |code: &'static str, extra: Option<(&'static str, String)>| {
        let mut params = vec![("key", key.to_string())];
        params.extend(extra);
        Err(CoreError::invalid(code, params))
    };
    if s.contains(['\n', '\r']) {
        return err("core.field.singleLine", None);
    }
    if no_spaces && s.contains(' ') {
        return err("core.field.noSpaces", None);
    }
    if s.chars().count() > max_len {
        return err("core.field.tooLong", Some(("max", max_len.to_string())));
    }
    Ok(Value::Text(s))
}

/// Horas (número con punto o coma, entre 0 y 10000), o `None` si no es válido. Acepta lo mismo que
/// `float()` de Python: exponentes, `inf`/`nan` (que quedan fuera de rango) y `_` entre dígitos.
pub fn parse_hours(text: &str) -> Option<f64> {
    let s = py::strip(text).replace(',', ".");
    let s = if s.contains('_') {
        let b = s.as_bytes();
        let ok = (0..b.len()).all(|i| {
            b[i] != b'_'
                || (i > 0 && i + 1 < b.len() && b[i - 1].is_ascii_digit() && b[i + 1].is_ascii_digit())
        });
        if !ok {
            return None;
        }
        s.replace('_', "")
    } else {
        s
    };
    let h: f64 = s.parse().ok()?;
    (0.0..=MAX_HOURS).contains(&h).then_some(h)
}

/// `format(x, "g")` de Python: 6 cifras significativas, sin ceros de más, notación científica fuera de
/// [1e-4, 1e6).
pub fn fmt_g(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    // El exponente que decide es el del número ya redondeado a 6 cifras (999999.5 → 1e+06).
    let sci = format!("{x:.5e}");
    let (mantissa, e) = sci.split_once('e').unwrap();
    let e: i32 = e.parse().unwrap();
    if !(-4..6).contains(&e) {
        let m = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    let decimals = (5 - e).max(0) as usize;
    let fixed = format!("{x:.decimals$}");
    if fixed.contains('.') { fixed.trim_end_matches('0').trim_end_matches('.').to_string() } else { fixed }
}

/// `AAAA-MM-DD` válida (ASCII, mes y día que existen) → (año, mes, día).
pub fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !(0..10).filter(|i| *i != 4 && *i != 7).all(|i| b[i].is_ascii_digit()) {
        return None;
    }
    let (y, m, d): (i32, u32, u32) = (s[..4].parse().ok()?, s[5..7].parse().ok()?, s[8..].parse().ok()?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (y >= 1 && (1..=days).contains(&d)).then_some((y, m, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_g_matches_python() {
        for (x, s) in [
            (12.5, "12.5"),
            (1000.0, "1000"),
            (0.1, "0.1"),
            (10000.0, "10000"),
            (8.25, "8.25"),
            (1e-5, "1e-05"),
            (1234567.0, "1.23457e+06"),
            (0.0001, "0.0001"),
            (3.0, "3"),
            (123456.0, "123456"),
        ] {
            assert_eq!(fmt_g(x), s, "{x}");
        }
    }

    #[test]
    fn dates() {
        assert_eq!(parse_date("2024-02-29"), Some((2024, 2, 29)));
        assert_eq!(parse_date("2026-02-29"), None);
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-1-01"), None);
    }

    #[test]
    fn hours() {
        assert_eq!(parse_hours("1_000"), Some(1000.0));
        assert_eq!(parse_hours("1__0"), None);
        assert_eq!(parse_hours("nan"), None);
    }
}
