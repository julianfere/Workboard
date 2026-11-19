//! Texto de los README: secciones por título, slug para nombres de carpeta y qué le falta a un plan.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use crate::tasks::TaskSection;
use crate::{Workflow, py};

static SECTION_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#{2,4})\s+(.*)").unwrap());
static HEADING_1_4_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#{1,4})\s").unwrap());
static HEADING_1_2_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,2}\s").unwrap());

/// Nombre apto para carpeta: sin acentos, minúsculas, `-` entre palabras, como mucho `max_len`.
/// Si no queda nada, `fallback`.
pub fn slugify(text: &str, max_len: usize, fallback: &str) -> String {
    let ascii: String = text.nfkd().filter(char::is_ascii).collect::<String>().to_lowercase();
    let mut slug = String::new();
    for c in ascii.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let cut: String = slug.chars().take(max_len).collect();
    let cut = cut.trim_end_matches('-');
    if cut.is_empty() { fallback.to_string() } else { cut.to_string() }
}

/// Contenido bajo el primer título `##`–`####` que contenga alguna palabra clave (en minúsculas),
/// hasta el próximo título del mismo nivel o superior.
pub fn extract_section(body: &str, keywords: &[&str]) -> String {
    let lines = py::splitlines(body, false);
    for (i, line) in lines.iter().enumerate() {
        let Some(m) = SECTION_RE.captures(line) else { continue };
        let title = m[2].to_lowercase();
        if !keywords.iter().any(|k| title.contains(k)) {
            continue;
        }
        let level = m[1].len();
        let out: Vec<&str> = lines[i + 1..]
            .iter()
            .take_while(|l| HEADING_1_4_RE.captures(l).is_none_or(|h| h[1].len() > level))
            .copied()
            .collect();
        return py::strip(&out.join("\n")).to_string();
    }
    String::new()
}

/// Lo que le falta a un README planificado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanGap {
    /// La sección de plan está vacía o solo tiene el texto de relleno.
    Plan,
    /// No hay ninguna tarea abierta.
    Tasks,
}

pub fn plan_gaps(wf: &Workflow, body: &str) -> Vec<PlanGap> {
    let heading = Regex::new(&format!(r"(?i)^##\s+{}\b", regex::escape(&wf.plan.heading))).expect("escaped");
    let lines = py::splitlines(body, false);
    let plan = lines.iter().position(|l| heading.is_match(l)).map(|i| {
        let section: Vec<&str> =
            lines[i + 1..].iter().take_while(|l| !HEADING_1_2_RE.is_match(l)).copied().collect();
        py::strip(&section.join("\n").replace(&wf.plan.placeholder, "")).to_string()
    });
    let mut gaps = Vec::new();
    if plan.is_none_or(|p| p.chars().count() < wf.plan.min_chars) {
        gaps.push(PlanGap::Plan);
    }
    let tasks = TaskSection::new(&wf.tasks.heading, wf.tasks.max_len);
    if !tasks.parse(body).iter().any(|t| !t.done) {
        gaps.push(PlanGap::Tasks);
    }
    gaps
}
