//! Tareas del README: las líneas `- [ ] texto` / `- [x] texto` de primer nivel bajo el título de tareas
//! del flujo (`## Pendientes` en classic-es), hasta el próximo título de nivel 1 o 2. Lo demás de la
//! sección (texto suelto, sub-ítems) no se toca.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::py;

static TASK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^- \[( |x|X)\] (.*)$").unwrap());
static TOP_HEADING_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,2}\s").unwrap());

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    pub text: String,
    pub done: bool,
}

/// Reglas de tareas de un flujo: el título de la sección y el largo máximo.
#[derive(Debug, Clone)]
pub struct TaskSection {
    heading: Regex,
    title: String,
    max_len: usize,
}

impl TaskSection {
    pub fn new(title: &str, max_len: usize) -> Self {
        let heading = Regex::new(&format!(r"(?i)^##\s+{}\s*$", regex::escape(title))).expect("escaped title");
        Self { heading, title: title.to_string(), max_len }
    }

    fn span(&self, lines: &[&str]) -> Option<(usize, usize)> {
        let start = lines.iter().position(|l| self.heading.is_match(py::strip(l)))?;
        let end =
            (start + 1..lines.len()).find(|&j| TOP_HEADING_RE.is_match(lines[j])).unwrap_or(lines.len());
        Some((start, end))
    }

    fn task_lines(&self, lines: &[&str]) -> Vec<usize> {
        self.span(lines).map_or(vec![], |(a, b)| (a + 1..b).filter(|&k| TASK_RE.is_match(lines[k])).collect())
    }

    pub fn parse(&self, body: &str) -> Vec<Task> {
        let lines = py::splitlines(body, false);
        self.task_lines(&lines).into_iter().map(|k| parse_line(lines[k])).collect()
    }

    /// Normaliza el texto de una tarea (espacios colapsados) y lo valida.
    pub fn clean(&self, text: &str) -> Result<String> {
        let text = py::split_ws(text).collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return Err(CoreError::invalid("core.task.empty", vec![]));
        }
        if text.chars().count() > self.max_len {
            return Err(CoreError::invalid("core.task.tooLong", vec![("max", self.max_len.to_string())]));
        }
        Ok(text)
    }

    /// Agrega una tarea abierta al final de la lista (crea la sección si no existe). `text` ya limpio.
    pub fn add(&self, body: &str, text: &str) -> String {
        let mut lines: Vec<String> = py::splitlines(body, false).into_iter().map(String::from).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        match self.span(&refs) {
            None => {
                while lines.last().is_some_and(|l| py::strip(l).is_empty()) {
                    lines.pop();
                }
                lines.extend([
                    String::new(),
                    format!("## {}", self.title),
                    String::new(),
                    format!("- [ ] {text}"),
                ]);
            }
            Some((start, _)) => {
                let at = self.task_lines(&refs).last().copied().unwrap_or(start) + 1;
                lines.insert(at, format!("- [ ] {text}"));
            }
        }
        join_lines(&lines)
    }

    /// Línea de la tarea `index`, verificando que su texto sea `expect` (si se pasa): si el README cambió,
    /// falla en vez de tocar otra tarea.
    fn locate(&self, lines: &[&str], index: i64, expect: Option<&str>) -> Result<usize> {
        let tasks = self.task_lines(lines);
        let k = usize::try_from(index)
            .ok()
            .and_then(|i| tasks.get(i).copied())
            .ok_or_else(|| CoreError::invalid("core.task.notFound", vec![]))?;
        if let Some(expect) = expect
            && parse_line(lines[k]).text != py::strip(expect)
        {
            return Err(CoreError::invalid("core.task.changed", vec![]));
        }
        Ok(k)
    }

    pub fn set(&self, body: &str, index: i64, done: bool, expect: Option<&str>) -> Result<String> {
        self.rewrite(body, index, expect, |line| {
            Some(format!("- [{}] {}", if done { 'x' } else { ' ' }, parse_line(line).text))
        })
    }

    pub fn remove(&self, body: &str, index: i64, expect: Option<&str>) -> Result<String> {
        self.rewrite(body, index, expect, |_| None)
    }

    /// Cambia el texto (conserva si está hecha). `text` ya limpio.
    pub fn edit(&self, body: &str, index: i64, text: &str, expect: Option<&str>) -> Result<String> {
        self.rewrite(body, index, expect, |line| {
            let mark = &TASK_RE.captures(line).expect("task line")[1];
            Some(format!("- [{mark}] {text}"))
        })
    }

    fn rewrite(
        &self,
        body: &str,
        index: i64,
        expect: Option<&str>,
        f: impl FnOnce(&str) -> Option<String>,
    ) -> Result<String> {
        let lines = py::splitlines(body, false);
        let k = self.locate(&lines, index, expect)?;
        let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        match f(lines[k]) {
            Some(new) => out[k] = new,
            None => {
                out.remove(k);
            }
        }
        Ok(join_lines(&out))
    }

    /// Tilda todas las tareas abiertas (al cerrar el trabajo). Sin abiertas devuelve el cuerpo tal cual.
    pub fn close_all(&self, body: &str) -> String {
        let lines = py::splitlines(body, false);
        let open: Vec<usize> =
            self.task_lines(&lines).into_iter().filter(|&k| !parse_line(lines[k]).done).collect();
        if open.is_empty() {
            return body.to_string();
        }
        let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        for k in open {
            out[k] = format!("- [x] {}", parse_line(lines[k]).text);
        }
        join_lines(&out)
    }

    /// Saca las tareas tildadas. Devuelve el cuerpo nuevo y los textos que salieron.
    pub fn clear_done(&self, body: &str) -> (String, Vec<String>) {
        let lines = py::splitlines(body, false);
        let done: Vec<usize> =
            self.task_lines(&lines).into_iter().filter(|&k| parse_line(lines[k]).done).collect();
        let texts = done.iter().map(|&k| parse_line(lines[k]).text).collect();
        let out: Vec<String> =
            lines.iter().enumerate().filter(|(k, _)| !done.contains(k)).map(|(_, l)| l.to_string()).collect();
        (join_lines(&out), texts)
    }
}

fn parse_line(line: &str) -> Task {
    let c = TASK_RE.captures(line).expect("task line");
    Task { text: py::strip(&c[2]).to_string(), done: &c[1] != " " }
}

fn join_lines(lines: &[String]) -> String {
    let mut out = lines.join("\n");
    out.push('\n');
    out
}
