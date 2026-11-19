//! Comandos de sesiones de agentes: la lista con su estado en vivo, el ciclo de vida, la terminal (para
//! clientes que consultan; la ventana se suscribe a la salida aparte) y Planear.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value as Json, json};
use wb_core::frontmatter::Value;
use wb_core::write::Change;
use wb_core::{StateCategory, Workspace};
use wb_sessions::{Env, Options, SessionManager, agents};

use crate::{Api, ApiError, ApiResult, Args, settings, to_json};

/// Variables para usar otros ejecutables del MCP y del hook (desarrollo, pruebas).
pub const MCP_BIN_ENV: &str = "WORKBOARD_MCP_BIN";
pub const HOOK_BIN_ENV: &str = "WORKBOARD_HOOK_BIN";

/// Ejecutable que viaja con la app: junto al exe actual (en desarrollo, `target/<perfil>/`).
fn sibling_bin(env: &str, name: &str) -> PathBuf {
    if let Some(p) = std::env::var_os(env).filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    // Los tests corren desde target/<perfil>/deps.
    [dir.join(&file), dir.parent().map(|d| d.join(&file)).unwrap_or_default()]
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| dir.join(file))
}

fn size(a: &Args) -> (u16, u16) {
    let n = |k: &str, d: u16| a.get(k).as_u64().map_or(d, |v| v.clamp(1, 1000) as u16);
    (n("cols", 120), n("rows", 30))
}

fn str_list(v: &Json) -> Vec<String> {
    v.as_array().map_or(vec![], |l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect())
}

impl Api {
    /// El manager de sesiones (se levanta la primera vez que hace falta).
    pub fn session_manager(&self) -> ApiResult<Arc<SessionManager>> {
        if let Some(m) = self.sessions.get() {
            return Ok(m.clone());
        }
        let config_dir = self.config_dir.clone();
        let bins = self.session_bins.lock().expect("bins").clone();
        let (mcp_bin, hook_bin) = bins
            .unwrap_or_else(|| (sibling_bin(MCP_BIN_ENV, "wb-mcp"), sibling_bin(HOOK_BIN_ENV, "wb-hook")));
        let mgr = SessionManager::start(Options {
            config_dir: self.config_dir.clone(),
            mcp_bin,
            hook_bin,
            env: Box::new(move || {
                let s = settings::load(&settings::path_in(&config_dir));
                let workflow = wb_plugins::resolve_workflow(&config_dir, &s.workflow)
                    .unwrap_or_else(|_| wb_core::Workflow::preset("default").expect("default preset"));
                let lang =
                    wb_i18n::resolve_lang(s.lang.as_deref().unwrap_or(wb_i18n::DEFAULT_LANG)).to_string();
                Env { workflow, lang, agents: s.agents }
            }),
        })?;
        Ok(self.sessions.get_or_init(|| mgr).clone())
    }

    /// Usa otros ejecutables del MCP y del hook (antes de la primera sesión).
    pub fn set_session_bins(&self, mcp: PathBuf, hook: PathBuf) {
        *self.session_bins.lock().expect("bins") = Some((mcp, hook));
    }

    /// Cierra las terminales al salir (las sesiones quedan pausadas).
    pub fn shutdown(&self) {
        if let Some(m) = self.sessions.get() {
            m.shutdown();
        }
    }

    /// Nombre para mostrar de un proyecto (la carpeta que contiene a la de trabajos, o la propia).
    fn project_name(root: &str) -> String {
        let p = Path::new(root);
        p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| root.to_string())
    }

    pub(crate) fn session_command(&self, cmd: &str, a: &Args) -> Option<ApiResult<Json>> {
        let out = match cmd {
            "sessions_rev" => self.session_manager().map(|m| json!(m.rev())),
            "sessions" => self.list_sessions(),
            "session_agents" => self.agents_info(),
            "set_agents" => self.set_agents(a),
            "session_create" => self.create_session(a),
            "session_resume" => self.with_ws(|m, ws| to_json(m.resume(ws, a.str("id")?, size(a))?)),
            "session_pause" => self.with_ws(|m, ws| to_json(m.pause(ws.root(), a.str("id")?)?)),
            "session_close" => self.with_ws(|m, ws| to_json(m.close(ws.root(), a.str("id")?)?)),
            "session_reopen" => self.with_ws(|m, ws| to_json(m.reopen(ws.root(), a.str("id")?)?)),
            "session_rename" => {
                self.with_ws(|m, ws| to_json(m.rename(ws.root(), a.str("id")?, a.str("name")?)?))
            }
            "session_pin" => {
                self.with_ws(|m, ws| to_json(m.pin(ws.root(), a.str("id")?, a.bool("pinned"))?))
            }
            "session_add_items" => {
                self.with_ws(|m, ws| to_json(m.add_items(ws, a.str("id")?, &str_list(a.get("items")))?))
            }
            "session_remove_item" => {
                self.with_ws(|m, ws| to_json(m.remove_item(ws, a.str("id")?, a.str("item")?)?))
            }
            "session_split" => {
                self.with_ws(|m, ws| to_json(m.split(ws, a.str("id")?, a.str("item")?, size(a))?))
            }
            "session_continue" => {
                self.with_ws(|m, ws| to_json(m.continue_in_new(ws, a.str("id")?, size(a))?))
            }
            "session_prompt" => self.session_prompt(a),
            "session_read" => self
                .session_manager()
                .and_then(|m| Ok(m.read_since(a.str("id")?, a.get("offset").as_u64().unwrap_or(0))?)),
            "session_write" => {
                self.session_manager().and_then(|m| to_json(m.write(a.str("id")?, a.str("data")?)?))
            }
            "session_resize" => self.session_manager().and_then(|m| {
                let (cols, rows) = size(a);
                to_json(m.resize(a.str("id")?, cols, rows)?)
            }),
            "plan_item" => a.str("name").and_then(|name| self.plan_item(name, size(a))),
            _ => return None,
        };
        Some(out)
    }

    fn with_ws(&self, f: impl FnOnce(&SessionManager, &Workspace) -> ApiResult<Json>) -> ApiResult<Json> {
        let m = self.session_manager()?;
        let ws = self.workspace()?;
        f(&m, &ws)
    }

    /// Sesiones de la carpeta abierta, las que corren en otros proyectos y las que se cerraron solas
    /// (todos sus items terminados) en esta vuelta, para ofrecer deshacer.
    fn list_sessions(&self) -> ApiResult<Json> {
        let m = self.session_manager()?;
        let root = self.settings().root;
        let (here, closed) = match self.workspace() {
            Ok(ws) => {
                let closed = m.auto_close(&ws)?;
                (m.sessions(ws.root()), closed)
            }
            Err(_) => (vec![], vec![]),
        };
        let others: Vec<Json> = m
            .running_elsewhere(root.as_deref().map(Path::new))
            .into_iter()
            .map(|v| {
                let project = Self::project_name(&v.session.root);
                let mut j = json!(v);
                j["project"] = json!(project);
                j
            })
            .collect();
        Ok(json!({"rev": m.rev(), "sessions": here, "others": others, "autoClosed": closed}))
    }

    fn agents_info(&self) -> ApiResult<Json> {
        let s = self.settings();
        let env: std::collections::BTreeMap<String, String> = std::env::vars().collect();
        let mut env = env;
        agents::ensure_node_on_path(&mut env);
        Ok(json!({
            "agents": agents::available(&s.agents.custom, &env),
            "default": s.agents.default_agent(),
            "custom": s.agents.custom,
            "planPrefix": s.agents.plan_prefix,
            "placeholders": agents::PLACEHOLDERS,
            "modes": agents::MODES,
        }))
    }

    fn set_agents(&self, a: &Args) -> ApiResult<Json> {
        let mut s = self.settings();
        let clean = |v: &Json| wb_core::py::split_ws(v.as_str().unwrap_or("")).collect::<Vec<_>>().join(" ");
        if let Some(c) = a.get("custom").as_object() {
            let c = Json::Object(c.clone());
            s.agents.custom = agents::CustomAgent {
                label: clean(&c["label"]),
                command: clean(&c["command"]),
                args: clean(&c["args"]),
                resume_args: clean(&c["resumeArgs"]),
            };
        }
        if let Some(p) = a.get("planPrefix").as_object() {
            s.agents.plan_prefix = p
                .iter()
                .filter(|(k, _)| agents::agent(k).is_some())
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .filter(|(_, v)| !v.trim().is_empty())
                .collect();
        }
        if let Some(d) = a.opt_str("default") {
            if agents::agent(d).is_none() {
                return Err(ApiError::new("session.agent.unknown", &[("agent", d)]));
            }
            s.agents.default = d.to_string();
        }
        if s.agents.default == agents::CUSTOM && !s.agents.custom.is_set() {
            s.agents.default = agents::DEFAULT_AGENT.into();
        }
        self.save_settings(s)?;
        self.agents_info()
    }

    fn remember_agent(&self, agent: &str) -> ApiResult<()> {
        let mut s = self.settings();
        if s.agents.default != agent {
            s.agents.default = agent.to_string();
            self.save_settings(s)?;
        }
        Ok(())
    }

    fn create_session(&self, a: &Args) -> ApiResult<Json> {
        let m = self.session_manager()?;
        let ws = self.workspace()?;
        let agent = a.opt_str("agent").filter(|s| !s.is_empty()).map(String::from);
        let agent = agent.unwrap_or_else(|| self.settings().agents.default_agent().to_string());
        let sess = m.create(
            &ws,
            &str_list(a.get("items")),
            a.opt_str("name").unwrap_or(""),
            a.opt_str("mode").unwrap_or("default"),
            &agent,
            false,
            a.opt_str("model"),
            None,
            size(a),
        )?;
        self.remember_agent(&agent)?;
        to_json(sess)
    }

    fn session_prompt(&self, a: &Args) -> ApiResult<Json> {
        let m = self.session_manager()?;
        let text = match a.str("kind")? {
            "handoff" => m.handoff_prompt(),
            "compact" => "/compact".to_string(),
            other => return Err(ApiError::new("session.unknownPrompt", &[("kind", other)])),
        };
        m.send_prompt(a.str("id")?, &text)?;
        Ok(Json::Null)
    }

    /// Planear: el item pasa al primer estado planificado, el README gana la sección de plan si no la
    /// tiene y se abre una sesión en modo plan con el pedido armado. El hook `Stop` valida el plan.
    fn plan_item(&self, name: &str, size: (u16, u16)) -> ApiResult<Json> {
        let m = self.session_manager()?;
        let ws = self.workspace()?;
        let wf = ws.workflow().clone();
        let planned = wf
            .states
            .iter()
            .find(|s| s.category == StateCategory::Planned)
            .ok_or_else(|| ApiError::new("error.noPlanState", &[]))?
            .id
            .clone();
        let heading = regex::Regex::new(&format!(r"(?im)^##\s+{}\s*$", regex::escape(&wf.plan.heading)))
            .map_err(ApiError::generic)?;
        let (status_key, plan_heading, placeholder) =
            (wf.fields.status.clone(), wf.plan.heading.clone(), wf.plan.placeholder.clone());
        ws.write_readme(name, |mut meta, mut body| {
            meta.insert(status_key.clone(), Value::Text(planned.clone()));
            if !heading.is_match(&body) {
                body = format!("{}\n\n## {plan_heading}\n\n{placeholder}\n", body.trim_end_matches('\n'));
            }
            Ok(Change::Parts(meta, body))
        })?;
        let it = ws.build_item(&ws.folder_of(name)?)?;
        let lang = wb_i18n::resolve_lang(self.settings().lang.as_deref().unwrap_or(wb_i18n::DEFAULT_LANG));
        let title = if it.title.is_empty() { name.to_string() } else { it.title.clone() };
        let params = [
            ("folder", name),
            ("title", title.as_str()),
            ("heading", plan_heading.as_str()),
            ("tasks", wf.tasks.heading.as_str()),
        ];
        let (prompt, session_name) = match it.ids.first() {
            Some(id) => {
                let mut p = params.to_vec();
                p.push(("id", id));
                (
                    wb_i18n::t(lang, "agent.plan.withId", &p),
                    wb_i18n::t(lang, "session.planName", &[("what", id)]),
                )
            }
            None => (
                wb_i18n::t(lang, "agent.plan.noId", &params),
                wb_i18n::t(lang, "session.planName", &[("what", name)]),
            ),
        };
        let agent = self.settings().agents.default_agent().to_string();
        let sess = m.create(
            &ws,
            &[name.to_string()],
            &session_name,
            "plan",
            &agent,
            true,
            None,
            Some(&prompt),
            size,
        )?;
        Ok(json!({"folder": name, "session": sess}))
    }

    /// Después de mover carpetas: las sesiones siguen a sus items (lo que va a eliminados no).
    pub(crate) fn sessions_moved(&self, ws: &Workspace, old: &str, new: &str) {
        if ws.workflow().is_ignored_folder(new.split('/').next().unwrap_or(new)) {
            return;
        }
        if let Ok(m) = self.session_manager() {
            m.renamed(ws, &[(old.to_string(), new.to_string())].into_iter().collect());
        }
    }
}
