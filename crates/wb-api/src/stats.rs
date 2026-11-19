//! Comandos de horas trabajadas, métricas por sesión, tools y Reflexionar (todo sale de las sesiones de
//! agentes de la raíz abierta), más exportar un texto (CSV) a la carpeta de descargas.

use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};
use wb_sessions::tools;

use crate::{Api, ApiError, ApiResult, Args, to_json};

/// Lo que se muestra del código de un script.
const PREVIEW_CHARS: usize = 200_000;

fn path_key(p: &str) -> String {
    if cfg!(windows) { p.replace('/', "\\").to_lowercase() } else { p.to_string() }
}

fn str_list(v: &Json) -> Vec<String> {
    v.as_array().map_or(vec![], |l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect())
}

/// Nombre de archivo sin caracteres que el sistema no acepta.
fn safe_file_name(name: &str) -> String {
    let clean: String =
        name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '-' } else { c }).collect();
    let clean = clean.trim().trim_matches('.').to_string();
    if clean.is_empty() { "export.csv".into() } else { clean }
}

impl Api {
    pub(crate) fn stats_command(&self, cmd: &str, a: &Args) -> Option<ApiResult<Json>> {
        let out = match cmd {
            "time_report" => self.time_report(),
            "stats_report" => self.stats_report(),
            "tools_overview" => self.tools_overview(),
            "tool_preview" => a.str("path").and_then(|p| self.tool_preview(p)),
            "tool_promote" => self.tool_promote(a),
            "tool_update" => (|| {
                let t = tools::update(
                    &self.config_dir,
                    a.str("name")?,
                    a.opt_str("description").unwrap_or(""),
                    a.opt_str("usage").unwrap_or(""),
                )?;
                Ok(t.to_json())
            })(),
            "tool_remove" => a.str("name").and_then(|n| to_json(tools::remove(&self.config_dir, n)?)),
            "reflect_items" => self.reflect_items(a),
            "apply_learnings" => self.apply_learnings(a),
            "save_export" => a.str("name").and_then(|n| self.save_export(n, a.str("text")?)),
            "reveal_export" => a.str("path").and_then(reveal_export),
            _ => return None,
        };
        Some(out)
    }

    fn time_report(&self) -> ApiResult<Json> {
        let ws = self.workspace()?;
        to_json(self.session_manager()?.time_report(ws.root()))
    }

    fn stats_report(&self) -> ApiResult<Json> {
        let ws = self.workspace()?;
        Ok(json!({"sessions": self.session_manager()?.stats_report(&ws)}))
    }

    fn tools_overview(&self) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let (promoted, candidates) = self.session_manager()?.tools_overview(&ws);
        Ok(json!({
            "tools": promoted.iter().map(tools::Tool::to_json).collect::<Vec<_>>(),
            "candidates": candidates,
        }))
    }

    /// Solo se leen scripts que escribió una sesión de esta raíz o copias de tools.
    fn known_script(&self, path: &str) -> ApiResult<PathBuf> {
        let ws = self.workspace()?;
        let (promoted, candidates) = self.session_manager()?.tools_overview(&ws);
        let key = path_key(path);
        let known = candidates.iter().flat_map(|c| c.paths.iter()).any(|p| path_key(p) == key)
            || promoted.iter().any(|t| path_key(&t.path.display().to_string()) == key);
        if !known {
            return Err(ApiError::new("tools.error.notCandidate", &[]));
        }
        Ok(PathBuf::from(path))
    }

    fn tool_preview(&self, path: &str) -> ApiResult<Json> {
        let p = self.known_script(path)?;
        let bytes = std::fs::read(&p).map_err(wb_core::error::CoreError::from)?;
        let code: String = String::from_utf8_lossy(&bytes).chars().take(PREVIEW_CHARS).collect();
        let (description, usage) = tools::describe(&p);
        Ok(json!({"code": code, "description": description, "usage": usage}))
    }

    /// Promueve un candidato (o vuelve a promover una tool cuyo script cambió).
    fn tool_promote(&self, a: &Args) -> ApiResult<Json> {
        let path = a.str("path")?;
        let name = a.str("name")?.trim().to_lowercase();
        let replace = a.bool("replace");
        let ws = self.workspace()?;
        let (promoted, candidates) = self.session_manager()?.tools_overview(&ws);
        let key = path_key(path);
        let cand = candidates.iter().find(|c| c.paths.iter().any(|p| path_key(p) == key));
        let existing = promoted.iter().find(|t| t.meta.name == name);
        let origin = match (cand, existing) {
            (Some(c), _) => json!({
                "sesiones": c.sessions.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
                "wis": c.items,
            }),
            (None, Some(t)) if replace && path_key(&t.path.display().to_string()) == key => {
                t.meta.origin.clone()
            }
            _ => return Err(ApiError::new("tools.error.notCandidate", &[])),
        };
        let description = a.opt_str("description").unwrap_or("");
        if description.trim().is_empty() {
            return Err(ApiError::new("tools.error.noDescription", &[]));
        }
        let t = tools::promote(
            &self.config_dir,
            Path::new(path),
            &name,
            description,
            a.opt_str("usage").unwrap_or(""),
            origin,
            replace,
        )?;
        Ok(t.to_json())
    }

    fn reflect_items(&self, a: &Args) -> ApiResult<Json> {
        let m = self.session_manager()?;
        let ws = self.workspace()?;
        let n = |k: &str, d: u16| a.get(k).as_u64().map_or(d, |v| v.clamp(1, 1000) as u16);
        let agent = self.settings().agents.default_agent().to_string();
        let (sess, evidence) =
            m.reflect(&ws, &str_list(a.get("names")), &agent, (n("cols", 120), n("rows", 30)))?;
        Ok(json!({"session": sess, "evidence": evidence.display().to_string()}))
    }

    /// Aplicar aprendizajes: abre una sesión normal (pide permiso para cada cambio) que aplica los candidatos
    /// elegidos de una nota de Reflexionar. Los items de la sesión son los vinculados a la nota.
    fn apply_learnings(&self, a: &Args) -> ApiResult<Json> {
        let ws = self.workspace()?;
        let notes = self.notes()?;
        let note = notes.read(a.str("id")?)?;
        let items = ws.list_items()?;
        let mut names: Vec<String> = vec![];
        for key in &note.items {
            if let Some(it) = items.iter().find(|i| &i.name == key || i.ids.contains(key))
                && !names.contains(&it.name)
            {
                names.push(it.name.clone());
            }
        }
        if names.is_empty() {
            return Err(ApiError::new("learn.noItems", &[]));
        }
        let lang = wb_i18n::resolve_lang(self.settings().lang.as_deref().unwrap_or(wb_i18n::DEFAULT_LANG));
        let which =
            wb_core::py::split_ws(a.get("which").as_str().unwrap_or("")).collect::<Vec<_>>().join(" ");
        let which = if which.is_empty() { wb_i18n::t(lang, "agent.learn.accepted", &[]) } else { which };
        let path = notes.folder().join(&note.id).display().to_string();
        let prompt = wb_i18n::t(
            lang,
            "agent.learn.prompt",
            &[
                ("which", &which),
                ("title", &note.title),
                ("path", &path),
                ("addTask", wb_sessions::names::ADD_TASK),
            ],
        );
        let short: String = note.title.chars().take(40).collect();
        let name = wb_i18n::t(lang, "learn.sessionName", &[("title", &short)]);
        let n = |k: &str, d: u16| a.get(k).as_u64().map_or(d, |v| v.clamp(1, 1000) as u16);
        let agent = self.settings().agents.default_agent().to_string();
        let sess = self.session_manager()?.create(
            &ws,
            &names,
            &name,
            "default",
            &agent,
            false,
            None,
            Some(&prompt),
            (n("cols", 120), n("rows", 30)),
        )?;
        Ok(json!({"session": sess}))
    }

    /// Exportar: el texto va a la carpeta de descargas, con BOM para que Excel lea bien los acentos.
    /// Devuelve la ruta (si existe, se agrega un número).
    fn save_export(&self, name: &str, text: &str) -> ApiResult<Json> {
        let dir = wb_platform::downloads_dir();
        std::fs::create_dir_all(&dir).map_err(wb_core::error::CoreError::from)?;
        let name = safe_file_name(name);
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (name.clone(), String::new()),
        };
        let mut path = dir.join(&name);
        let mut i = 2;
        while path.exists() {
            path = dir.join(format!("{stem} ({i}){ext}"));
            i += 1;
        }
        let mut data = "\u{feff}".as_bytes().to_vec();
        data.extend_from_slice(text.as_bytes());
        std::fs::write(&path, data).map_err(wb_core::error::CoreError::from)?;
        Ok(json!(path.display().to_string()))
    }
}

/// Muestra en el explorador un archivo exportado (solo de la carpeta de descargas).
fn reveal_export(path: &str) -> ApiResult<Json> {
    let p = PathBuf::from(path);
    let inside = p.parent().is_some_and(|d| {
        path_key(&d.display().to_string()) == path_key(&wb_platform::downloads_dir().display().to_string())
    });
    if !inside {
        return Err(ApiError::new("error.badArgument", &[("name", "path")]));
    }
    wb_platform::reveal_path(&p).map_err(wb_core::error::CoreError::from)?;
    Ok(Json::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_names_are_safe() {
        assert_eq!(safe_file_name("horas 2026/11:x.csv"), "horas 2026-11-x.csv");
        assert_eq!(safe_file_name("  "), "export.csv");
    }
}
