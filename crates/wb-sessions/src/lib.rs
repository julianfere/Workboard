//! Sesiones de agentes embebidas. Una sesión = uno o más items (o un grupo); la app genera el id, la
//! corre en una pseudo-terminal dentro de la ventana y la retoma después. Siempre en el padre de la
//! carpeta de trabajos: el agente se mueve solo entre los repos.
//!
//! - `registry`: qué sesiones hay por raíz (lo lee también el MCP para saber qué puede escribir cada una).
//! - `agents`: la línea de comando de cada agente (Claude Code, Copilot CLI, comando propio).
//! - `pty`: el proceso y su salida.
//! - `transcript`: el peso de una sesión de Claude Code.
//!
//! El estado en vivo (trabajando, te necesita, reposo) llega por los hooks del CLI: `wb-hook <Evento>`
//! postea a un HTTP local con token y devuelve lo que la app le contesta al CLI.

pub mod agents;
pub mod candidates;
pub mod pty;
pub mod reflect;
pub mod registry;
pub mod stats;
pub mod timelog;
pub mod transcript;

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use wb_core::error::{CoreError, ErrorKind, Result};
use wb_core::frontmatter::{self, Value};
use wb_core::text::{PlanGap, plan_gaps};
use wb_core::workspace::{Item, README};
use wb_core::write::Change;
use wb_core::{StateCategory, Workflow, Workspace};
use wb_mcp::names;

pub use agents::{AGENTS, CustomAgent};
pub use pty::{PtyEvent, Sink};
pub use registry::{Registry, Session, State};
pub use wb_mcp::tools;

/// Eventos de Claude Code que avisa el hook.
pub const HOOK_EVENTS: [&str; 6] =
    ["SessionStart", "UserPromptSubmit", "PostToolUse", "Notification", "Stop", "SessionEnd"];
/// Veces que el hook `Stop` devuelve a trabajar una sesión de planificación con el plan a medias.
const PLAN_BLOCKS_MAX: u32 = 2;
/// Escrituras del MCP en el README (`<id>.events.jsonl`).
const README_EVENTS: [&str; 3] = ["set_estado", "add_task", "complete_task"];
/// Segundos antes de volver a buscar un transcript que todavía no existe.
const MISS_TTL: Duration = Duration::from_secs(30);
/// Caché del índice id de item → carpeta (aviso de otro item en el pedido).
const INDEX_TTL: Duration = Duration::from_secs(60);
/// Variables de "soy una sesión de Claude Code" que hereda la app si se la lanza desde una terminal de
/// Claude. Pasadas a la pty, el claude embebido se cree sesión hija y no guarda el transcript.
const CLAUDE_SESSION_VARS: [&str; 7] = [
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SSE_PORT",
];
const CLAUDE_SESSION_PREFIXES: [&str; 2] = ["CLAUDE_CODE_SESSION_", "CLAUDE_CODE_MESSAGING_"];

/// Preferencias de agentes (`settings.json` → `agents`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentSettings {
    /// Agente por defecto (el último elegido).
    pub default: String,
    pub custom: CustomAgent,
    /// Con qué empieza el pedido de una sesión de planificación, por agente (una skill, por ejemplo).
    pub plan_prefix: BTreeMap<String, String>,
}

impl AgentSettings {
    pub fn default_agent(&self) -> &str {
        match agents::agent(&self.default) {
            Some(a) if a.id != agents::CUSTOM || self.custom.is_set() => a.id,
            _ => agents::DEFAULT_AGENT,
        }
    }
}

/// Lo que la sesión necesita saber de la app en cada momento.
pub struct Env {
    pub workflow: Workflow,
    pub lang: String,
    pub agents: AgentSettings,
}

pub type EnvFn = Box<dyn Fn() -> Env + Send + Sync>;

pub struct Options {
    pub config_dir: PathBuf,
    /// Ejecutables del MCP (`wb-mcp`) y del hook (`wb-hook`).
    pub mcp_bin: PathBuf,
    pub hook_bin: PathBuf,
    pub env: EnvFn,
}

/// Estado en vivo de una sesión con hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Live {
    Starting,
    Working,
    NeedsYou,
    Idle,
    Ended,
    /// Agente sin hooks: solo se sabe que corre.
    Running,
    Paused,
    Closed,
}

#[derive(Debug, Clone)]
struct LiveInfo {
    status: Live,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherItem {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherItems {
    pub ts: f64,
    pub items: Vec<OtherItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Weight {
    pub ctx: u64,
    pub prompts: usize,
    pub level: transcript::Level,
    #[serde(flatten)]
    pub tail: transcript::TailInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommend: Option<transcript::Recommendation>,
}

/// Una sesión para la UI: lo del registro más el estado en vivo.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    #[serde(flatten)]
    pub session: Session,
    pub running: bool,
    pub status: Live,
    pub status_detail: String,
    pub hooks: bool,
    pub weight: Option<Weight>,
    pub other_items: Option<OtherItems>,
}

type TranscriptStamp = (SystemTime, u64);
type ItemIndex = BTreeMap<String, String>;

fn err(kind: ErrorKind, code: &'static str, params: Vec<(&'static str, String)>) -> CoreError {
    CoreError::new(kind, code, params)
}

fn not_found() -> CoreError {
    err(ErrorKind::NotFound, "session.notFound", vec![])
}

fn epoch_now() -> f64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn io_err(e: std::io::Error) -> CoreError {
    e.into()
}

/// Un item es de la sesión si está en su lista o es parte de un grupo de la lista.
fn in_session(name: &str, items: &[String]) -> bool {
    items.iter().any(|w| name == w || name.starts_with(&format!("{w}/")))
}

/// Nombre nuevo de un item tras mover carpetas: la carpeta misma o la de su grupo.
fn moved_name(mapping: &BTreeMap<String, String>, name: &str) -> String {
    if let Some(n) = mapping.get(name) {
        return n.clone();
    }
    mapping
        .iter()
        .find_map(|(old, new)| name.strip_prefix(&format!("{old}/")).map(|rest| format!("{new}/{rest}")))
        .unwrap_or_else(|| name.to_string())
}

/// Comillas para un comando de hook: sirve en Git Bash y en cmd.
fn quote(p: &Path) -> String {
    format!("\"{}\"", p.display().to_string().replace('\\', "/"))
}

/// Ruta absoluta en el formato de las reglas de permisos de Claude Code: `//c/Users/...` en Windows.
fn rule_path(p: &Path) -> String {
    let p = dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let mut s = p.display().to_string().replace('\\', "/");
    if s.len() > 1 && s.as_bytes()[1] == b':' {
        s = format!("/{}{}", s[..1].to_lowercase(), &s[2..]);
    }
    format!("/{s}")
}

pub struct SessionManager {
    config_dir: PathBuf,
    mcp_bin: PathBuf,
    hook_bin: PathBuf,
    env: EnvFn,
    token: String,
    hook_port: u16,
    me: Weak<SessionManager>,
    ptys: Mutex<HashMap<String, Arc<pty::Pty>>>,
    live: Mutex<HashMap<String, LiveInfo>>,
    /// Avisos para el agente que salen en su próximo `UserPromptSubmit`.
    pending: Mutex<HashMap<String, Vec<String>>>,
    plan_blocks: Mutex<HashMap<String, u32>>,
    other_items: Mutex<HashMap<String, OtherItems>>,
    transcripts: Mutex<HashMap<String, std::result::Result<PathBuf, Instant>>>,
    /// Peso por sesión, con la (fecha, tamaño) del transcript de la que salió.
    weights: Mutex<HashMap<String, (TranscriptStamp, Weight)>>,
    /// Por raíz: cuándo se armó y el índice id de item → carpeta.
    indexes: Mutex<HashMap<String, (Instant, ItemIndex)>>,
    /// Sube con cada cambio que la UI tiene que ver (la UI lo consulta en vez de pedir la lista entera).
    rev: AtomicU64,
}

impl SessionManager {
    /// Crea el manager y levanta el receptor de hooks (HTTP local con token).
    pub fn start(opts: Options) -> std::io::Result<Arc<Self>> {
        let server =
            tiny_http::Server::http("127.0.0.1:0").map_err(|e| std::io::Error::other(e.to_string()))?;
        let port = server.server_addr().to_ip().map_or(0, |a| a.port());
        let mgr = Arc::new_cyclic(|me| Self {
            config_dir: opts.config_dir,
            mcp_bin: opts.mcp_bin,
            hook_bin: opts.hook_bin,
            env: opts.env,
            token: uuid::Uuid::new_v4().simple().to_string(),
            hook_port: port,
            me: me.clone(),
            ptys: Mutex::default(),
            live: Mutex::default(),
            pending: Mutex::default(),
            plan_blocks: Mutex::default(),
            other_items: Mutex::default(),
            transcripts: Mutex::default(),
            weights: Mutex::default(),
            indexes: Mutex::default(),
            rev: AtomicU64::new(1),
        });
        let weak = Arc::downgrade(&mgr);
        std::thread::Builder::new().name("wb-hooks".into()).spawn(move || serve_hooks(server, weak))?;
        Ok(mgr)
    }

    /// Puerto y token del receptor de hooks.
    pub fn hook_endpoint(&self) -> (u16, String) {
        (self.hook_port, self.token.clone())
    }

    pub fn rev(&self) -> u64 {
        self.rev.load(Ordering::SeqCst)
    }

    fn notify(&self) {
        self.rev.fetch_add(1, Ordering::SeqCst);
    }

    fn env(&self) -> Env {
        (self.env)()
    }

    fn t(&self, lang: &str, key: &str, params: &[(&str, &str)]) -> String {
        wb_i18n::t(lang, key, params)
    }

    fn open_ws(&self, root: &str, wf: Workflow) -> Result<Workspace> {
        Workspace::open(Path::new(root), wf, Some(self.config_dir.join("backups")))
    }

    fn registry_of(&self, sid: &str) -> Option<Registry> {
        Registry::find(&self.config_dir, sid)
    }

    fn session(&self, sid: &str) -> Option<(Registry, Session)> {
        let reg = self.registry_of(sid)?;
        let s = reg.get(sid)?;
        Some((reg, s))
    }

    fn pty(&self, sid: &str) -> Option<Arc<pty::Pty>> {
        self.ptys.lock().expect("ptys").get(sid).cloned()
    }

    fn running(&self, sid: &str) -> bool {
        self.pty(sid).is_some_and(|p| p.alive())
    }

    fn queue(&self, sid: &str, msg: String) {
        self.pending.lock().expect("pending").entry(sid.to_string()).or_default().push(msg);
    }

    // ---------- registro de actividad ----------

    pub fn activity_path(&self, sid: &str) -> PathBuf {
        self.config_dir.join("sessions").join(format!("{sid}.activity.jsonl"))
    }

    /// Historial de la sesión para medir tiempo trabajado: una línea por evento con la hora (epoch) y los
    /// items que tenía en ese momento. Es un extra: nunca frena nada.
    fn activity(&self, sid: &str, ev: &str, items: Option<&[String]>, extra: Json) {
        let items: Vec<String> = match items {
            Some(i) => i.to_vec(),
            None => self.session(sid).map(|(_, s)| s.items).unwrap_or_default(),
        };
        let mut row = json!({"ts": (epoch_now() * 10.0).round() / 10.0, "ev": ev, "items": items});
        if let (Some(r), Some(e)) = (row.as_object_mut(), extra.as_object()) {
            r.extend(e.clone());
        }
        let path = self.activity_path(sid);
        let _ = (|| -> std::io::Result<()> {
            use std::io::Write;
            std::fs::create_dir_all(path.parent().expect("parent"))?;
            writeln!(std::fs::OpenOptions::new().create(true).append(true).open(&path)?, "{row}")
        })();
    }

    // ---------- hooks ----------

    /// Un evento del CLI. Devuelve lo que `wb-hook` imprime (el CLI lo suma al contexto del turno o, en
    /// `Stop`, lo toma como decisión de seguir trabajando).
    pub fn hook(&self, event: &str, sid: &str, payload: &Json) -> String {
        if sid.is_empty() {
            return String::new();
        }
        if event == "Stop"
            && let Some(reason) = self.check_plan(sid, payload)
        {
            self.activity(sid, "check", None, json!({"ok": false, "reason": reason}));
            return json!({"decision": "block", "reason": reason}).to_string();
        }
        if event == "PostToolUse"
            && payload["tool_name"].as_str().is_some_and(|n| transcript::EDIT_TOOLS.contains(&n))
        {
            self.track_repo(sid, payload);
        }
        let status = match event {
            "Notification" if payload["notification_type"] == "idle_prompt" => Live::Idle,
            "Notification" => Live::NeedsYou,
            "SessionStart" | "Stop" => Live::Idle,
            "UserPromptSubmit" | "PostToolUse" => Live::Working,
            "SessionEnd" => Live::Ended,
            _ => return String::new(),
        };
        let others = if event == "UserPromptSubmit" {
            self.other_items_in(sid, payload["prompt"].as_str().unwrap_or(""))
        } else {
            vec![]
        };
        let detail = payload["message"].as_str().unwrap_or("").to_string();
        let prev = self
            .live
            .lock()
            .expect("live")
            .insert(sid.to_string(), LiveInfo { status, detail })
            .map(|l| l.status);
        let mut out = String::new();
        if event == "UserPromptSubmit" {
            self.plan_blocks.lock().expect("blocks").remove(sid);
            let mut parts = self.pending.lock().expect("pending").remove(sid).unwrap_or_default();
            if !others.is_empty() {
                let lang = self.env().lang;
                let list: Vec<String> = others
                    .iter()
                    .map(|o| self.t(&lang, "agent.otherItem.entry", &[("id", &o.id), ("folder", &o.name)]))
                    .collect();
                let key = if others.len() == 1 { "agent.otherItem.one" } else { "agent.otherItem.many" };
                parts.push(self.t(&lang, key, &[("list", &list.join(", "))]));
                let names: Vec<&str> = others.iter().map(|o| o.name.as_str()).collect();
                self.activity(sid, "otherItem", None, json!({"others": names}));
                self.other_items
                    .lock()
                    .expect("others")
                    .insert(sid.to_string(), OtherItems { ts: epoch_now(), items: others });
            }
            out = parts.join("\n\n");
        }
        // PostToolUse llega con cada herramienta: solo se anota si reabre trabajo (tras un permiso).
        if event != "PostToolUse" || prev != Some(Live::Working) {
            let ev = if event == "PostToolUse" { "resume" } else { event };
            // Las métricas cuentan los pedidos del agente (permiso o respuesta), no los avisos de reposo.
            let extra = if status == Live::NeedsYou { json!({"st": "needsYou"}) } else { json!({}) };
            self.activity(sid, ev, None, extra);
        }
        if (event == "UserPromptSubmit" || event == "Stop")
            && let Some(reg) = self.registry_of(sid)
        {
            let _ = reg.modify(sid, |s| s.last_activity = registry::now());
        }
        self.notify();
        out
    }

    /// Validación de las sesiones de planificación al terminar un turno. Si el agente escribió el plan
    /// pero no cargó tareas (o al revés), se lo devuelve a trabajar con lo que falta, como mucho
    /// `PLAN_BLOCKS_MAX` veces. Si falta todo, probablemente está preguntando algo: no se lo frena; queda
    /// un aviso para su próximo turno.
    fn check_plan(&self, sid: &str, payload: &Json) -> Option<String> {
        let (_, sess) = self.session(sid)?;
        if !sess.plan {
            return None;
        }
        let env = self.env();
        let ws = self.open_ws(&sess.root, env.workflow).ok()?;
        let wf = ws.workflow();
        let mut gaps: Vec<(String, Vec<PlanGap>)> = vec![];
        for name in &sess.items {
            let Ok(folder) = ws.folder_of(name) else { continue };
            let readme = folder.join(README);
            if wf.is_group_folder(name.rsplit('/').next().unwrap_or(name)) || !readme.exists() {
                continue;
            }
            let Ok(text) = frontmatter::read(&readme) else { continue };
            let (meta, body) = frontmatter::split(&text);
            let status = meta.get(&wf.fields.status).and_then(Value::as_text).unwrap_or("");
            if wf.state(status).is_some_and(|s| s.category == StateCategory::Planned) {
                let g = plan_gaps(wf, &body);
                if !g.is_empty() {
                    gaps.push((name.clone(), g));
                }
            }
        }
        if gaps.is_empty() {
            self.activity(sid, "check", None, json!({"ok": true}));
            return None;
        }
        let lang = &env.lang;
        let gap_text = |g: &PlanGap| match g {
            PlanGap::Plan => self.t(lang, "agent.gap.plan", &[("heading", &wf.plan.heading)]),
            PlanGap::Tasks => self.t(lang, "agent.gap.tasks", &[("tasks", &wf.tasks.heading)]),
        };
        let and = self.t(lang, "agent.and", &[]);
        let text = gaps
            .iter()
            .map(|(name, g)| format!("{name}: {}", g.iter().map(gap_text).collect::<Vec<_>>().join(&and)))
            .collect::<Vec<_>>()
            .join("; ");
        let partial = gaps.iter().all(|(_, g)| g.len() == 1);
        let mut blocks = self.plan_blocks.lock().expect("blocks");
        let n = blocks.get(sid).copied().unwrap_or(0);
        if partial && !payload["stop_hook_active"].as_bool().unwrap_or(false) && n < PLAN_BLOCKS_MAX {
            blocks.insert(sid.to_string(), n + 1);
            return Some(self.t(
                lang,
                "agent.planIncomplete",
                &[("gaps", &text), ("heading", &wf.plan.heading), ("addTask", names::ADD_TASK)],
            ));
        }
        drop(blocks);
        self.queue(sid, self.t(lang, "agent.planMissing", &[("gaps", &text)]));
        self.activity(sid, "check", None, json!({"ok": false, "reason": text}));
        None
    }

    /// Repo = primera carpeta bajo `cwd`. Todas las sesiones corren en el mismo lugar, así que el repo
    /// sale de lo que edita cada una.
    fn track_repo(&self, sid: &str, payload: &Json) {
        let Some((reg, sess)) = self.session(sid) else { return };
        let inp = &payload["tool_input"];
        let Some(path) = inp["file_path"].as_str().or_else(|| inp["notebook_path"].as_str()) else { return };
        let cwd = PathBuf::from(&sess.cwd);
        let mut p = PathBuf::from(path);
        if !p.is_absolute() {
            p = PathBuf::from(payload["cwd"].as_str().unwrap_or(&sess.cwd)).join(p);
        }
        let canon = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let Ok(rel) = canon(&p).strip_prefix(canon(&cwd)).map(Path::to_path_buf) else { return };
        let parts: Vec<String> =
            rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
        let root_name = Path::new(&sess.root).file_name().map(|n| n.to_string_lossy().to_string());
        if parts.len() < 2 || Some(&parts[0]) == root_name.as_ref() {
            return; // archivo suelto en cwd o dentro de la carpeta de trabajos: no es un repo
        }
        if !sess.repos.contains(&parts[0]) {
            let _ = reg.modify(sid, |s| s.repos.push(parts[0].clone()));
        }
    }

    // ---------- otro item en el pedido ----------

    fn item_index(&self, ws: &Workspace) -> BTreeMap<String, String> {
        let key = registry::normcase(ws.root());
        if let Some((t, idx)) = self.indexes.lock().expect("indexes").get(&key)
            && t.elapsed() < INDEX_TTL
        {
            return idx.clone();
        }
        let mut index = BTreeMap::new();
        for it in ws.list_items().unwrap_or_default() {
            for id in &it.ids {
                index.entry(id.clone()).or_insert_with(|| it.name.clone());
            }
        }
        self.indexes.lock().expect("indexes").insert(key, (Instant::now(), index.clone()));
        index
    }

    /// Items de la raíz que el pedido nombra por id y no son de la sesión. Solo cuentan ids que existen
    /// (un número de build u otro id no avisa).
    pub fn other_items_in(&self, sid: &str, prompt: &str) -> Vec<OtherItem> {
        let digits = regex::Regex::new(r"\d+").expect("regex");
        let Some((_, sess)) = self.session(sid) else { return vec![] };
        let env = self.env();
        let nums: std::collections::BTreeSet<&str> = digits
            .find_iter(prompt)
            .map(|m| m.as_str())
            .filter(|n| env.workflow.is_valid_item_id(n))
            .collect();
        if nums.is_empty() {
            return vec![];
        }
        let Ok(ws) = self.open_ws(&sess.root, env.workflow) else { return vec![] };
        let index = self.item_index(&ws);
        nums.into_iter()
            .filter_map(|n| index.get(n).map(|name| (n, name)))
            .filter(|(_, name)| !in_session(name, &sess.items))
            .map(|(n, name)| OtherItem { id: n.to_string(), name: name.clone() })
            .collect()
    }

    // ---------- archivos que recibe cada sesión ----------

    fn context_file(&self, sess: &Session, ws: &Workspace, lang: &str) -> Result<PathBuf> {
        let all = ws.list_items()?;
        let tools = self.tools();
        let text = wb_mcp::context::session_context(
            ws.workflow(),
            lang,
            ws.root(),
            &sess.name,
            &sess.items,
            &all,
            &tools,
        );
        let path = self.config_dir.join("ctx").join(&sess.id).join("AGENTS.md");
        std::fs::create_dir_all(path.parent().expect("parent"))?;
        std::fs::write(&path, text)?;
        Ok(path)
    }

    fn tools(&self) -> Vec<Json> {
        let ctx =
            wb_mcp::Ctx { config_dir: self.config_dir.clone(), session: None, write_all: false, root: None };
        wb_mcp::registered_tools(&ctx)
    }

    fn mcp_config(&self, sess: &Session) -> Result<PathBuf> {
        if !self.mcp_bin.is_file() {
            let name = self.mcp_bin.display().to_string();
            return Err(err(ErrorKind::NotFound, "session.missingBinary", vec![("path", name)]));
        }
        let env = json!({
            "WORKBOARD_SESSION": sess.id,
            "WORKBOARD_ROOT": sess.root,
            "WORKBOARD_CONFIG_DIR": self.config_dir.display().to_string(),
        });
        let cmd = self.mcp_bin.display().to_string();
        let server = if sess.agent == agents::COPILOT {
            json!({"type": "local", "command": cmd, "args": [], "env": env, "tools": ["*"]})
        } else {
            json!({"type": "stdio", "command": cmd, "args": [], "env": env})
        };
        let path = self.config_dir.join("agents").join(format!("mcp-{}.json", sess.id));
        std::fs::create_dir_all(path.parent().expect("parent"))?;
        let text = serde_json::to_string_pretty(&json!({"mcpServers": {"workboard": server}}))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(&path, text)?;
        Ok(path)
    }

    /// Settings de Claude para la sesión: los hooks y los permisos que da trabajar desde Workboard (el
    /// MCP `workboard` y editar dentro de las carpetas de sus items sin preguntar).
    fn claude_settings(&self, sess: &Session) -> Result<PathBuf> {
        if !self.hook_bin.is_file() {
            let name = self.hook_bin.display().to_string();
            return Err(err(ErrorKind::NotFound, "session.missingBinary", vec![("path", name)]));
        }
        let mut hooks = serde_json::Map::new();
        for ev in HOOK_EVENTS {
            let mut entry = json!({"hooks": [{"type": "command", "command": format!("{} {ev}", quote(&self.hook_bin)), "timeout": 5}]});
            if ev == "PostToolUse" {
                entry["matcher"] = json!("*");
            }
            hooks.insert(ev.to_string(), json!([entry]));
        }
        let root = PathBuf::from(&sess.root);
        let mut allow = vec!["mcp__workboard".to_string()];
        for name in &sess.items {
            let folder = name.split('/').fold(root.clone(), |p, part| p.join(part));
            for tool in ["Edit", "Write"] {
                allow.push(format!("{tool}({}/**)", rule_path(&folder)));
            }
        }
        for d in ["reflect", "review", "sessions", "tools"] {
            allow.push(format!("Read({}/**)", rule_path(&self.config_dir.join(d))));
        }
        if let Some(projects) = transcript::projects_dir() {
            allow.push(format!("Read({}/**)", rule_path(&projects)));
        }
        let path = self.config_dir.join("claude").join(format!("settings-{}.json", sess.id));
        std::fs::create_dir_all(path.parent().expect("parent"))?;
        let text = serde_json::to_string_pretty(&json!({"hooks": hooks, "permissions": {"allow": allow}}))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(&path, text)?;
        Ok(path)
    }

    fn has_transcript(&self, sid: &str) -> bool {
        transcript::projects_dir().and_then(|p| transcript::find(&p, sid)).is_some()
    }

    // ---------- peso (sesión larga) ----------

    fn transcript_path(&self, sid: &str) -> Option<PathBuf> {
        let mut cache = self.transcripts.lock().expect("transcripts");
        match cache.get(sid) {
            Some(Ok(p)) if p.exists() => return Some(p.clone()),
            Some(Err(t)) if t.elapsed() < MISS_TTL => return None,
            _ => {}
        }
        let found = transcript::projects_dir().and_then(|d| transcript::find(&d, sid));
        cache.insert(sid.to_string(), found.clone().ok_or_else(Instant::now));
        found
    }

    fn prompt_count(&self, sid: &str) -> usize {
        std::fs::read_to_string(self.activity_path(sid))
            .map(|t| t.lines().filter(|l| l.contains("\"UserPromptSubmit\"")).count())
            .unwrap_or(0)
    }

    /// Contexto de la última respuesta y si conviene cortar. Se recalcula solo si el transcript cambió.
    pub fn weight(&self, sess: &Session) -> Option<Weight> {
        let path = self.transcript_path(&sess.id)?;
        let meta = std::fs::metadata(&path).ok()?;
        let key = (meta.modified().ok()?, meta.len());
        let cached = self
            .weights
            .lock()
            .expect("weights")
            .get(&sess.id)
            .filter(|(k, _)| *k == key)
            .map(|(_, w)| w.clone());
        let mut w = match cached {
            Some(w) => w,
            None => {
                let tail = transcript::tail_info(&path, meta.len(), README);
                let w = Weight {
                    ctx: tail.ctx,
                    prompts: self.prompt_count(&sess.id),
                    level: transcript::level(tail.ctx),
                    tail,
                    recommend: None,
                };
                self.weights.lock().expect("weights").insert(sess.id.clone(), (key, w.clone()));
                w
            }
        };
        if w.level != transcript::Level::Normal {
            w.recommend = Some(transcript::recommend(
                &w.tail,
                self.last_readme_event(&sess.id),
                self.state_changed(sess),
                registry::epoch(&sess.last_activity),
                epoch_now(),
            ));
        }
        Some(w)
    }

    /// Última escritura del MCP de esta sesión en el README (epoch).
    fn last_readme_event(&self, sid: &str) -> f64 {
        let path = self.config_dir.join("sessions").join(format!("{sid}.events.jsonl"));
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Json>(l).ok())
            .filter(|d| d["tipo"].as_str().is_some_and(|t| README_EVENTS.contains(&t)))
            .map(|d| registry::epoch(d["ts"].as_str().unwrap_or("")))
            .fold(0.0, f64::max)
    }

    /// ¿Cambió el estado de algún item de la sesión desde que se creó (por el agente o desde la UI)?
    fn state_changed(&self, sess: &Session) -> bool {
        let since = registry::epoch(&sess.created);
        let root = registry::normcase(Path::new(&sess.root));
        std::fs::read_to_string(self.config_dir.join("state_log.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Json>(l).ok())
            .any(|r| {
                r["session"] == sess.id.as_str()
                    || (r["root"] == root.as_str()
                        && r["ts"].as_f64().unwrap_or(0.0) >= since
                        && in_session(r["wi"].as_str().unwrap_or(""), &sess.items))
            })
    }

    // ---------- horas, métricas, tools y Reflexionar ----------

    /// Tiempo trabajado por item y por día de las sesiones de la raíz.
    pub fn time_report(&self, root: &Path) -> timelog::TimeReport {
        let list: Vec<(Session, bool)> = Registry::of_root(&self.config_dir, root)
            .load()
            .into_iter()
            .map(|s| {
                let running = self.running(&s.id);
                (s, running)
            })
            .collect();
        timelog::report(&list, &self.config_dir, transcript::projects_dir().as_deref(), epoch_now())
    }

    /// Métricas por sesión de la raíz.
    pub fn stats_report(&self, ws: &Workspace) -> Vec<stats::SessionStats> {
        let list = Registry::of_root(&self.config_dir, ws.root()).load();
        stats::report(ws, &list, &self.config_dir, transcript::projects_dir().as_deref())
    }

    /// Tools promovidas y scripts de las sesiones de la raíz que conviene promover.
    pub fn tools_overview(&self, ws: &Workspace) -> (Vec<tools::Tool>, Vec<candidates::Candidate>) {
        let promoted = tools::list(&self.config_dir);
        let cands = candidates::candidates(&self.stats_report(ws), &promoted);
        (promoted, cands)
    }

    /// Reflexionar: arma la evidencia de las sesiones de esos items y abre una sesión en modo plan que
    /// propone mejoras y las deja en una nota. Nunca aplica nada. Devuelve la sesión y la evidencia.
    pub fn reflect(
        &self,
        ws: &Workspace,
        names: &[String],
        agent: &str,
        size: (u16, u16),
    ) -> Result<(Session, PathBuf)> {
        let mut items: Vec<String> = vec![];
        for n in names {
            let it = ws.build_item(&ws.folder_of(n)?)?;
            if !items.contains(&it.name) {
                items.push(it.name);
            }
        }
        if items.is_empty() {
            return Err(err(ErrorKind::Invalid, "session.noItems", vec![]));
        }
        let rows = self.stats_report(ws);
        if !rows.iter().any(|r| reflect::touches(&r.items, &items)) {
            return Err(err(ErrorKind::Invalid, "reflect.noSessions", vec![]));
        }
        let lang = self.env().lang;
        let extra = reflect::instruction_paths(ws.root());
        let text = reflect::evidence_text(ws, &items, &rows, &self.config_dir, &extra, &lang);
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = self.config_dir.join("reflect").join(format!("{stamp}.md"));
        std::fs::create_dir_all(path.parent().expect("parent")).map_err(io_err)?;
        std::fs::write(&path, text).map_err(io_err)?;
        let label = if items.len() > 3 {
            let n = (items.len() - 3).to_string();
            self.t(&lang, "reflect.more", &[("list", &items[..3].join(", ")), ("n", &n)])
        } else {
            items.join(", ")
        };
        let title = self.t(&lang, "agent.reflect.noteTitle", &[("label", &label)]);
        let prompt = self.t(
            &lang,
            "agent.reflect.prompt",
            &[
                ("label", &label),
                ("evidence", &path.display().to_string()),
                ("item", &items[0]),
                ("title", &title),
                ("addNote", names::ADD_NOTE),
            ],
        );
        let name = self.t(&lang, "reflect.sessionName", &[("label", &label)]);
        let sess = self.create(ws, &items, &name, "plan", agent, false, None, Some(&prompt), size)?;
        Ok((sess, path))
    }

    // ---------- lista ----------

    fn view(&self, s: Session) -> SessionView {
        let def = agents::agent(&s.agent).copied().unwrap_or(agents::AGENTS[0]);
        let running = self.running(&s.id);
        let live = self.live.lock().expect("live").get(&s.id).cloned();
        let status = if !running {
            if s.state == State::Closed { Live::Closed } else { Live::Paused }
        } else if def.hooks {
            live.as_ref().map_or(Live::Starting, |l| l.status)
        } else {
            Live::Running
        };
        let weight = if s.state != State::Closed && def.transcript { self.weight(&s) } else { None };
        SessionView {
            running,
            status,
            status_detail: if running { live.map(|l| l.detail).unwrap_or_default() } else { String::new() },
            hooks: def.hooks,
            weight,
            other_items: self.other_items.lock().expect("others").get(&s.id).cloned(),
            session: s,
        }
    }

    /// Sesiones de la raíz con su estado en vivo, la de actividad más reciente primero. Las activas sin
    /// proceso pasan a pausadas.
    pub fn sessions(&self, root: &Path) -> Vec<SessionView> {
        let reg = Registry::of_root(&self.config_dir, root);
        let stale: Vec<String> = reg
            .load()
            .into_iter()
            .filter(|s| s.state == State::Active && !self.running(&s.id))
            .map(|s| s.id)
            .collect();
        if !stale.is_empty() {
            let _ = reg.update(|ss| {
                for s in ss.iter_mut().filter(|s| stale.contains(&s.id)) {
                    s.state = State::Paused;
                }
            });
        }
        let mut out: Vec<SessionView> = reg.load().into_iter().map(|s| self.view(s)).collect();
        out.sort_by(|a, b| {
            let key = |v: &SessionView| {
                if v.session.last_activity.is_empty() {
                    v.session.created.clone()
                } else {
                    v.session.last_activity.clone()
                }
            };
            key(b).cmp(&key(a))
        });
        out
    }

    /// Sesiones con proceso vivo de otras raíces: siguen corriendo al cambiar de proyecto.
    pub fn running_elsewhere(&self, root: Option<&Path>) -> Vec<SessionView> {
        let here = root.map(registry::normcase).unwrap_or_default();
        let ids: Vec<String> = self.ptys.lock().expect("ptys").keys().cloned().collect();
        ids.into_iter()
            .filter(|id| self.running(id))
            .filter_map(|id| self.session(&id).map(|(_, s)| s))
            .filter(|s| registry::normcase(Path::new(&s.root)) != here)
            .map(|s| self.view(s))
            .collect()
    }

    /// Cierra las sesiones sin proceso cuyos items están todos cerrados (salvo las reabiertas a mano).
    /// Devuelve las que cerró, para ofrecer deshacer.
    pub fn auto_close(&self, ws: &Workspace) -> Result<Vec<String>> {
        let reg = Registry::of_root(&self.config_dir, ws.root());
        let items = ws.list_items()?;
        let wf = ws.workflow();
        let closed = |name: &str| {
            items.iter().find(|it| it.name == name).is_some_and(|it| {
                it.meta.get(&wf.fields.status).and_then(Value::as_text).is_some_and(|s| wf.is_closed(s))
            })
        };
        let done: Vec<String> = reg
            .load()
            .into_iter()
            .filter(|s| {
                s.state != State::Closed && !s.keep_open && !s.items.is_empty() && !self.running(&s.id)
            })
            .filter(|s| s.items.iter().all(|n| closed(n)))
            .map(|s| s.id)
            .collect();
        if !done.is_empty() {
            reg.update(|ss| {
                for s in ss.iter_mut().filter(|s| done.contains(&s.id)) {
                    s.state = State::Closed;
                }
            })
            .map_err(io_err)?;
            self.notify();
        }
        Ok(done)
    }

    // ---------- ciclo de vida ----------

    /// Trazabilidad en el item: el id de la sesión en el frontmatter (`fields.sessions`).
    fn link_readme(&self, ws: &Workspace, name: &str, sid: &str) -> Result<()> {
        let key = ws.workflow().fields.sessions.clone();
        ws.write_readme(name, |mut meta, body| {
            let mut list = meta.get(&key).map(Value::as_list).unwrap_or_default();
            if !list.iter().any(|s| s == sid) {
                list.push(sid.to_string());
            }
            meta.insert(key.clone(), Value::List(list));
            Ok(Change::Parts(meta, body))
        })?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        ws: &Workspace,
        items: &[String],
        name: &str,
        mode: &str,
        agent: &str,
        plan: bool,
        model: Option<&str>,
        prompt: Option<&str>,
        size: (u16, u16),
    ) -> Result<Session> {
        if items.is_empty() {
            return Err(err(ErrorKind::Invalid, "session.noItems", vec![]));
        }
        if !agents::MODES.contains(&mode) {
            return Err(err(ErrorKind::Invalid, "session.badMode", vec![("mode", mode.into())]));
        }
        let agent =
            if agent.is_empty() { self.env().agents.default_agent().to_string() } else { agent.to_string() };
        if agents::agent(&agent).is_none() {
            return Err(err(ErrorKind::Invalid, "session.agent.unknown", vec![("agent", agent)]));
        }
        for n in items {
            ws.folder_of(n)?;
        }
        let root = ws.root();
        let name = wb_core::py::split_ws(name).collect::<Vec<_>>().join(" ");
        let sess = Session {
            id: uuid::Uuid::new_v4().to_string(),
            name: if name.is_empty() { items[0].clone() } else { name.chars().take(80).collect() },
            root: root.display().to_string(),
            cwd: root.parent().unwrap_or(root).display().to_string(),
            items: items.to_vec(),
            state: State::Active,
            mode: mode.into(),
            agent,
            plan,
            model: model.filter(|m| !m.is_empty()).map(String::from),
            created: registry::now(),
            last_activity: registry::now(),
            ..Default::default()
        };
        let reg = Registry::of_root(&self.config_dir, root);
        reg.update(|ss| ss.push(sess.clone())).map_err(io_err)?;
        for n in items {
            self.link_readme(ws, n, &sess.id)?;
        }
        if let Err(e) = self.launch(&sess, ws, agents::Start::New, prompt, size) {
            let _ = reg.modify(&sess.id, |s| s.state = State::Paused);
            return Err(e);
        }
        Ok(sess)
    }

    fn launch(
        &self,
        sess: &Session,
        ws: &Workspace,
        start: agents::Start,
        prompt: Option<&str>,
        size: (u16, u16),
    ) -> Result<()> {
        let env_info = self.env();
        let mut env: BTreeMap<String, String> = std::env::vars()
            .filter(|(k, _)| {
                let up = k.to_uppercase();
                up != "WORKBOARD_MCP_WRITE"
                    && !CLAUDE_SESSION_VARS.contains(&up.as_str())
                    && !CLAUDE_SESSION_PREFIXES.iter().any(|p| up.starts_with(p))
            })
            .collect();
        env.insert("WORKBOARD_SESSION".into(), sess.id.clone());
        env.insert("WORKBOARD_ROOT".into(), sess.root.clone());
        env.insert("WORKBOARD_CONFIG_DIR".into(), self.config_dir.display().to_string());
        env.insert("WORKBOARD_HOOK_PORT".into(), self.hook_port.to_string());
        env.insert("WORKBOARD_HOOK_TOKEN".into(), self.token.clone());
        if !cfg!(windows) {
            env.entry("TERM".into()).or_insert_with(|| "xterm-256color".into());
            env.entry("COLORTERM".into()).or_insert_with(|| "truecolor".into());
        }
        agents::ensure_node_on_path(&mut env);

        let ctx = self.context_file(sess, ws, &env_info.lang)?;
        let mcp = self.mcp_config(sess)?;
        let settings = if sess.agent == agents::CLAUDE { Some(self.claude_settings(sess)?) } else { None };
        let files =
            agents::LaunchFiles { context: &ctx, mcp_config: &mcp, claude_settings: settings.as_deref() };
        let prefix = env_info.agents.plan_prefix.get(&sess.agent).cloned().unwrap_or_default();
        let (argv, extra) =
            agents::build_command(sess, &files, start, prompt, &prefix, &env_info.agents.custom, &env)?;
        env.extend(extra);

        let weak = self.me.clone();
        let sid = sess.id.clone();
        let argv: Vec<OsString> = argv;
        let p = pty::Pty::spawn(&argv, Path::new(&sess.cwd), &env, size.0, size.1, move |_| {
            if let Some(m) = weak.upgrade() {
                m.on_exit(&sid);
            }
        })
        .map_err(|e| err(ErrorKind::Io, "session.launchFailed", vec![("message", e.to_string())]))?;
        let old = self.ptys.lock().expect("ptys").insert(sess.id.clone(), p);
        if let Some(old) = old {
            old.kill();
        }
        self.live
            .lock()
            .expect("live")
            .insert(sess.id.clone(), LiveInfo { status: Live::Starting, detail: String::new() });
        self.activity(&sess.id, "launch", Some(&sess.items), json!({"agent": sess.agent}));
        self.notify();
        Ok(())
    }

    fn on_exit(&self, sid: &str) {
        let had = self.live.lock().expect("live").remove(sid);
        if had.is_some() {
            self.activity(sid, "exit", None, json!({}));
        }
        if let Some(reg) = self.registry_of(sid) {
            let _ = reg.update(|ss| {
                for s in ss.iter_mut().filter(|s| s.id == sid && s.state == State::Active) {
                    s.state = State::Paused;
                }
            });
        }
        self.notify();
    }

    pub fn resume(&self, ws: &Workspace, sid: &str, size: (u16, u16)) -> Result<Session> {
        let reg = Registry::of_root(&self.config_dir, ws.root());
        let sess = reg.get(sid).ok_or_else(not_found)?;
        if self.running(sid) {
            return Ok(sess);
        }
        let sess = reg
            .modify(sid, |s| {
                s.state = State::Active;
                s.last_activity = registry::now();
            })
            .map_err(io_err)?
            .ok_or_else(not_found)?;
        // Claude sin conversación guardada no tiene qué retomar: arranca con el mismo id. Copilot retoma o
        // crea con el mismo --session-id.
        let start = if sess.agent != agents::CLAUDE || self.has_transcript(sid) {
            agents::Start::Resume
        } else {
            agents::Start::New
        };
        if let Err(e) = self.launch(&sess, ws, start, None, size) {
            let _ = reg.modify(sid, |s| s.state = State::Paused);
            return Err(e);
        }
        Ok(sess)
    }

    pub fn pause(&self, root: &Path, sid: &str) -> Result<()> {
        if let Some(p) = self.pty(sid) {
            p.kill();
        }
        Registry::of_root(&self.config_dir, root)
            .modify(sid, |s| {
                if s.state != State::Closed {
                    s.state = State::Paused;
                }
            })
            .map_err(io_err)?;
        self.notify();
        Ok(())
    }

    pub fn close(&self, root: &Path, sid: &str) -> Result<()> {
        if let Some(p) = self.ptys.lock().expect("ptys").remove(sid) {
            p.kill();
        }
        Registry::of_root(&self.config_dir, root).modify(sid, |s| s.state = State::Closed).map_err(io_err)?;
        self.notify();
        Ok(())
    }

    /// Vuelve a la lista una sesión cerrada (queda pausada) y ya no se cierra sola.
    pub fn reopen(&self, root: &Path, sid: &str) -> Result<()> {
        Registry::of_root(&self.config_dir, root)
            .modify(sid, |s| {
                if s.state == State::Closed {
                    s.state = State::Paused;
                }
                s.keep_open = true;
            })
            .map_err(io_err)?;
        self.notify();
        Ok(())
    }

    pub fn rename(&self, root: &Path, sid: &str, name: &str) -> Result<String> {
        let name: String =
            wb_core::py::split_ws(name).collect::<Vec<_>>().join(" ").chars().take(80).collect();
        if name.is_empty() {
            return Err(err(ErrorKind::Invalid, "session.emptyName", vec![]));
        }
        let reg = Registry::of_root(&self.config_dir, root);
        reg.modify(sid, |s| s.name = name.clone()).map_err(io_err)?.ok_or_else(not_found)?;
        self.notify();
        Ok(name)
    }

    pub fn pin(&self, root: &Path, sid: &str, pinned: bool) -> Result<()> {
        let reg = Registry::of_root(&self.config_dir, root);
        reg.modify(sid, |s| s.pinned = pinned).map_err(io_err)?.ok_or_else(not_found)?;
        self.notify();
        Ok(())
    }

    fn item_message(&self, lang: &str, key: &str, params: &[(&str, &str)]) -> String {
        self.t(lang, key, params)
    }

    /// Suma items a una sesión. El MCP los puede escribir enseguida (lee el registro en cada llamada) y
    /// el agente se entera en su próximo pedido.
    pub fn add_items(&self, ws: &Workspace, sid: &str, names: &[String]) -> Result<Session> {
        let reg = Registry::of_root(&self.config_dir, ws.root());
        let sess = reg.get(sid).filter(|s| s.state != State::Closed).ok_or_else(not_found)?;
        let all = ws.list_items()?;
        let new: Vec<String> = names
            .iter()
            .filter(|n| !sess.items.contains(n) && all.iter().any(|it| &it.name == *n))
            .cloned()
            .collect();
        if new.is_empty() {
            return Ok(sess);
        }
        let sess = reg.modify(sid, |s| s.items.extend(new.clone())).map_err(io_err)?.ok_or_else(not_found)?;
        for n in &new {
            self.link_readme(ws, n, sid)?;
        }
        self.activity(sid, "items", Some(&sess.items), json!({}));
        let lang = self.env().lang;
        self.context_file(&sess, ws, &lang)?;
        let all = ws.list_items()?;
        let mut lines = vec![
            self.item_message(&lang, "agent.itemsAdded", &[("count", &new.len().to_string())]),
            String::new(),
        ];
        for n in &new {
            lines.extend(wb_mcp::context::item_lines(ws.workflow(), &lang, ws.root(), n, &all, "##"));
        }
        self.queue(sid, lines.join("\n"));
        self.notify();
        Ok(sess)
    }

    /// Saca un item de la sesión (sin bifurcar la conversación): deja de poder escribirlo.
    pub fn remove_item(&self, ws: &Workspace, sid: &str, name: &str) -> Result<Session> {
        let reg = Registry::of_root(&self.config_dir, ws.root());
        let sess = reg.get(sid).ok_or_else(not_found)?;
        if !sess.items.iter().any(|n| n == name) {
            return Err(err(ErrorKind::NotFound, "session.notInSession", vec![("item", name.into())]));
        }
        if sess.items.len() < 2 {
            return Err(err(ErrorKind::Invalid, "session.onlyItem", vec![]));
        }
        let sess =
            reg.modify(sid, |s| s.items.retain(|n| n != name)).map_err(io_err)?.ok_or_else(not_found)?;
        self.activity(sid, "items", Some(&sess.items), json!({}));
        let lang = self.env().lang;
        self.context_file(&sess, ws, &lang)?;
        self.queue(sid, self.item_message(&lang, "agent.itemRemoved", &[("item", name)]));
        self.notify();
        Ok(sess)
    }

    /// Separa un item de una sesión con varios: sesión nueva solo con ese item que hereda la conversación
    /// (si el agente sabe bifurcar). La original lo pierde.
    pub fn split(&self, ws: &Workspace, sid: &str, name: &str, size: (u16, u16)) -> Result<Session> {
        let reg = Registry::of_root(&self.config_dir, ws.root());
        let parent = reg.get(sid).ok_or_else(not_found)?;
        if !parent.items.iter().any(|n| n == name) {
            return Err(err(ErrorKind::NotFound, "session.notInSession", vec![("item", name.into())]));
        }
        if parent.items.len() < 2 {
            return Err(err(ErrorKind::Invalid, "session.nothingToSplit", vec![]));
        }
        let fork = agents::agent(&parent.agent).is_some_and(|a| a.fork);
        if fork && !self.has_transcript(sid) {
            return Err(err(ErrorKind::Invalid, "session.noConversation", vec![]));
        }
        let it = ws.list_items()?.into_iter().find(|it| it.name == name);
        let child_name = match &it {
            Some(Item { ids, title, .. }) if !ids.is_empty() => {
                let t = if title.is_empty() { name } else { title };
                format!("{} · {t}", ids[0]).chars().take(70).collect()
            }
            _ => name.to_string(),
        };
        let child = Session {
            id: uuid::Uuid::new_v4().to_string(),
            name: child_name,
            root: parent.root.clone(),
            cwd: parent.cwd.clone(),
            items: vec![name.to_string()],
            state: State::Active,
            mode: parent.mode.clone(),
            agent: parent.agent.clone(),
            model: parent.model.clone(),
            created: registry::now(),
            last_activity: registry::now(),
            fork_of: fork.then(|| sid.to_string()),
            ..Default::default()
        };
        reg.update(|ss| {
            for s in ss.iter_mut().filter(|s| s.id == sid) {
                s.items.retain(|n| n != name);
            }
            ss.push(child.clone());
        })
        .map_err(io_err)?;
        self.link_readme(ws, name, &child.id)?;
        let parent = reg.get(sid).ok_or_else(not_found)?;
        self.activity(sid, "items", Some(&parent.items), json!({}));
        let lang = self.env().lang;
        self.context_file(&parent, ws, &lang)?;
        self.queue(
            sid,
            self.item_message(&lang, "agent.itemSplit", &[("item", name), ("name", &child.name)]),
        );
        let start = if fork { agents::Start::Fork(sid) } else { agents::Start::New };
        self.launch(&child, ws, start, None, size)?;
        Ok(child)
    }

    /// Carpetas movidas (agrupar, sacar, eliminar, renombrar): `{viejo: nuevo}`. Actualiza el registro (el
    /// MCP sigue dejando escribir) y avisa a las sesiones que las tenían.
    pub fn renamed(&self, ws: &Workspace, mapping: &BTreeMap<String, String>) {
        if mapping.is_empty() {
            return;
        }
        let reg = Registry::of_root(&self.config_dir, ws.root());
        let touched = reg
            .update(|ss| {
                let mut touched = vec![];
                for s in ss.iter_mut() {
                    let mut new: Vec<String> = vec![];
                    for w in &s.items {
                        let m = moved_name(mapping, w);
                        if !new.contains(&m) {
                            new.push(m);
                        }
                    }
                    if new != s.items {
                        s.items = new;
                        touched.push(s.id.clone());
                    }
                }
                touched
            })
            .unwrap_or_default();
        if touched.is_empty() {
            return;
        }
        let lang = self.env().lang;
        let moves: Vec<String> = mapping
            .iter()
            .map(|(old, new)| {
                let path = new.split('/').fold(ws.root().to_path_buf(), |p, part| p.join(part));
                format!("- {old} → {new}  ({})", path.display())
            })
            .collect();
        for sid in touched {
            self.activity(&sid, "rename", None, json!({"map": mapping}));
            self.queue(&sid, format!("{}\n{}", self.t(&lang, "agent.itemsMoved", &[]), moves.join("\n")));
        }
        self.notify();
    }

    /// Escribe un pedido fijo en la terminal, como si lo tipeara el usuario. Solo en reposo: en medio de
    /// un turno se mezclaría con lo que está haciendo.
    pub fn send_prompt(&self, sid: &str, text: &str) -> Result<()> {
        let p = self
            .pty(sid)
            .filter(|p| p.alive())
            .ok_or_else(|| err(ErrorKind::Invalid, "session.notRunning", vec![]))?;
        let idle = self.live.lock().expect("live").get(sid).is_some_and(|l| l.status == Live::Idle);
        if !idle {
            return Err(err(ErrorKind::Invalid, "session.busy", vec![]));
        }
        let text = wb_core::py::split_ws(text).collect::<Vec<_>>().join(" ");
        p.write(text.as_bytes()).map_err(io_err)?;
        // El Enter va aparte: junto con el texto, la terminal lo toma como un pegado y no lo envía.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            let _ = p.write(b"\r");
        });
        Ok(())
    }

    /// Pedido de traspaso: que el agente deje el README al día antes de cortar.
    pub fn handoff_prompt(&self) -> String {
        let lang = self.env().lang;
        self.t(&lang, "agent.handoff", &[("update", names::UPDATE_ITEM), ("addTask", names::ADD_TASK)])
    }

    /// Cierra una sesión larga y abre otra con los mismos items, modo, agente y modelo, que arranca
    /// retomando desde el README. La cerrada se puede reabrir.
    pub fn continue_in_new(&self, ws: &Workspace, sid: &str, size: (u16, u16)) -> Result<Session> {
        let old = Registry::of_root(&self.config_dir, ws.root()).get(sid).ok_or_else(not_found)?;
        if old.items.is_empty() {
            return Err(err(ErrorKind::Invalid, "session.noItems", vec![]));
        }
        let lang = self.env().lang;
        let suffix = self.t(&lang, "agent.continued.suffix", &[]);
        let base = old.name.strip_suffix(&format!(" {suffix}")).unwrap_or(&old.name);
        let base: String = base.chars().take(62).collect();
        self.close(ws.root(), sid)?;
        self.activity(sid, "continued", None, json!({}));
        let prompt = self.t(&lang, "agent.continue", &[("name", &old.name), ("session", names::SESSION)]);
        self.create(
            ws,
            &old.items,
            &format!("{base} {suffix}"),
            &old.mode,
            &old.agent,
            old.plan,
            old.model.as_deref(),
            Some(&prompt),
            size,
        )
    }

    // ---------- terminal ----------

    pub fn attach(&self, sid: &str, sink: Sink) -> Result<()> {
        let p = self.pty(sid).ok_or_else(|| err(ErrorKind::NotFound, "session.noProcess", vec![]))?;
        p.attach(sink);
        Ok(())
    }

    /// Salida desde `offset` en base64 (para clientes que consultan: el servidor de desarrollo).
    pub fn read_since(&self, sid: &str, offset: u64) -> Result<Json> {
        use base64::Engine;
        let p = self.pty(sid).ok_or_else(|| err(ErrorKind::NotFound, "session.noProcess", vec![]))?;
        let (next, data, exited) = p.read_since(offset);
        Ok(json!({
            "offset": next,
            "data": base64::engine::general_purpose::STANDARD.encode(data),
            "exited": exited.is_some(),
            "code": exited.flatten(),
        }))
    }

    pub fn write(&self, sid: &str, data: &str) -> Result<()> {
        if let Some(p) = self.pty(sid) {
            p.write(data.as_bytes()).map_err(io_err)?;
        }
        Ok(())
    }

    pub fn resize(&self, sid: &str, cols: u16, rows: u16) -> Result<()> {
        if let Some(p) = self.pty(sid) {
            p.resize(cols, rows).map_err(io_err)?;
        }
        Ok(())
    }

    /// Al cerrar la app: matar solo nuestras terminales; las sesiones quedan pausadas para retomar.
    pub fn shutdown(&self) {
        let all: Vec<(String, Arc<pty::Pty>)> = self.ptys.lock().expect("ptys").drain().collect();
        for (sid, p) in all {
            if p.alive() && self.live.lock().expect("live").remove(&sid).is_some() {
                self.activity(&sid, "exit", None, json!({}));
            }
            p.kill();
            if let Some(reg) = self.registry_of(&sid) {
                let _ = reg.update(|ss| {
                    for s in ss.iter_mut().filter(|s| s.id == sid && s.state == State::Active) {
                        s.state = State::Paused;
                    }
                });
            }
        }
    }
}

/// Receptor de hooks: `POST /hook/<evento>` con `X-Workboard-Token`. El cuerpo es el JSON que el CLI le
/// pasa al hook; la respuesta es texto plano que el hook imprime tal cual.
fn serve_hooks(server: tiny_http::Server, mgr: Weak<SessionManager>) {
    for mut req in server.incoming_requests() {
        let Some(m) = mgr.upgrade() else { return };
        let url = req.url().to_string();
        let token_ok =
            req.headers().iter().any(|h| h.field.equiv("X-Workboard-Token") && h.value.as_str() == m.token);
        let header_sid = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("X-Workboard-Session"))
            .map(|h| h.value.to_string())
            .unwrap_or_default();
        if *req.method() != tiny_http::Method::Post || !url.starts_with("/hook/") || !token_ok {
            let _ = req.respond(tiny_http::Response::empty(403));
            continue;
        }
        let mut body = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::Read::take(req.as_reader(), 1_000_000), &mut body);
        let payload: Json = serde_json::from_slice(&body).unwrap_or(json!({}));
        // El id con el que se lanzó (variable de entorno del hook) manda: tras un `/clear` el CLI cambia el suyo.
        let sid = if header_sid.is_empty() {
            payload["session_id"].as_str().unwrap_or_default().to_string()
        } else {
            header_sid
        };
        let event = url["/hook/".len()..].to_string();
        let out = m.hook(&event, &sid, &payload);
        let _ = req.respond(tiny_http::Response::from_string(out));
    }
}
