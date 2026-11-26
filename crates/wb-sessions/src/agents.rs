//! Agentes que pueden correr una sesión: Claude Code, GitHub Copilot CLI o un comando propio. Todos
//! reciben lo mismo (el contexto de la sesión en un archivo y el MCP `workboard`); cada adaptador solo
//! resuelve cómo se le pasa eso a su CLI.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wb_core::error::{CoreError, ErrorKind, Result};

use crate::registry::Session;

pub const DEFAULT_AGENT: &str = "claude";
pub const CLAUDE: &str = "claude";
pub const COPILOT: &str = "copilot";
pub const CUSTOM: &str = "custom";
/// Modos de permisos que entiende la app.
pub const MODES: [&str; 3] = ["default", "plan", "acceptEdits"];
/// Placeholders de la plantilla de argumentos del comando propio.
pub const PLACEHOLDERS: [&str; 7] =
    ["session_id", "name", "prompt", "context_file", "context_dir", "mcp_config", "cwd"];

/// Lo que sabe hacer cada agente.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct AgentDef {
    pub id: &'static str,
    /// Ejecutable a buscar en el PATH ("" = el comando propio).
    pub exe: &'static str,
    /// Avisa por hooks trabajando / te necesita / reposo.
    pub hooks: bool,
    /// Puede bifurcar una conversación (Separar hereda la conversación).
    pub fork: bool,
    /// Lee el transcript para el peso de la sesión.
    pub transcript: bool,
    /// Con qué empieza por defecto el pedido de una sesión de planificación (`/go ` es la skill de
    /// planificación de Claude, como en la v1). El usuario lo cambia en Ajustes.
    pub plan_prefix: &'static str,
}

pub const AGENTS: [AgentDef; 3] = [
    AgentDef { id: CLAUDE, exe: "claude", hooks: true, fork: true, transcript: true, plan_prefix: "/go " },
    AgentDef { id: COPILOT, exe: "copilot", hooks: false, fork: false, transcript: false, plan_prefix: "" },
    AgentDef { id: CUSTOM, exe: "", hooks: false, fork: false, transcript: false, plan_prefix: "" },
];

pub fn agent(id: &str) -> Option<&'static AgentDef> {
    AGENTS.iter().find(|a| a.id == id)
}

/// Prefijos de planificación por defecto (los agentes que tienen uno).
pub fn default_plan_prefixes() -> BTreeMap<String, String> {
    AGENTS
        .iter()
        .filter(|a| !a.plan_prefix.is_empty())
        .map(|a| (a.id.to_string(), a.plan_prefix.to_string()))
        .collect()
}

/// Comando propio (`settings.json` → `agents.custom`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomAgent {
    pub label: String,
    pub command: String,
    /// Argumentos con `{placeholders}`.
    pub args: String,
    /// Argumentos para retomar (si no, los mismos).
    pub resume_args: String,
}

impl CustomAgent {
    pub fn is_set(&self) -> bool {
        !self.command.trim().is_empty()
    }
}

/// Un agente para elegir al crear una sesión, con si está instalado.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub id: &'static str,
    /// Solo el comando propio con etiqueta propia (los demás, y el propio sin etiqueta, se traducen en la UI).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    pub found: bool,
    pub hooks: bool,
    pub fork: bool,
}

fn err(code: &'static str, params: Vec<(&'static str, String)>) -> CoreError {
    CoreError::new(ErrorKind::Invalid, code, params)
}

// ---------- entorno y ejecutables ----------

/// Carpetas donde puede estar Node. fnm y nvm-windows lo ponen en el PATH solo en las terminales que
/// corren su `env`: abierta con doble clic, la app hereda un PATH sin Node y los paquetes npm globales
/// (copilot) no aparecen.
fn node_dirs(env: &BTreeMap<String, String>) -> Vec<PathBuf> {
    let get = |k: &str| env.get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = get("USERPROFILE").or_else(|| get("HOME")).unwrap_or_default();
    let mut fnm: Vec<PathBuf> = get("FNM_DIR").into_iter().collect();
    fnm.push(get("APPDATA").unwrap_or_else(|| home.join("AppData/Roaming")).join("fnm"));
    fnm.push(get("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData/Local")).join("fnm"));
    let mut dirs: Vec<PathBuf> = fnm.iter().map(|d| d.join("aliases").join("default")).collect();
    dirs.extend(get("NVM_SYMLINK"));
    dirs.push(get("ProgramFiles").unwrap_or_else(|| PathBuf::from(r"C:\Program Files")).join("nodejs"));
    let node = if cfg!(windows) { "node.exe" } else { "bin/node" };
    dirs.into_iter().filter(|d| d.join(node).exists()).collect()
}

const PATH_SEP: char = if cfg!(windows) { ';' } else { ':' };

/// Clave de la variable PATH tal como está en el entorno (en Windows es `Path`).
fn path_key(env: &BTreeMap<String, String>) -> String {
    env.keys().find(|k| k.eq_ignore_ascii_case("PATH")).cloned().unwrap_or_else(|| "PATH".into())
}

/// Si `npx` no está en el PATH, suma la carpeta de Node que encuentre.
pub fn ensure_node_on_path(env: &mut BTreeMap<String, String>) {
    let key = path_key(env);
    let path = env.get(&key).cloned().unwrap_or_default();
    if which("npx", &path).is_some() {
        return;
    }
    if let Some(dir) = node_dirs(env).into_iter().next() {
        let dir = if cfg!(windows) { dir } else { dir.join("bin") };
        env.insert(key, format!("{}{PATH_SEP}{path}", dir.display()));
    }
}

/// Busca un ejecutable en el PATH (con las extensiones de `PATHEXT` en Windows).
pub fn which(cmd: &str, path: &str) -> Option<PathBuf> {
    wb_platform::which(cmd, path)
}

/// argv para lanzar `cmd` en una pty. Los paquetes npm globales en Windows son un `.cmd` que la pty no
/// puede ejecutar directo: se lanza `node <script>` (lo mismo que hace el shim).
pub fn resolve_exe(cmd: &str, env: &BTreeMap<String, String>) -> Result<Option<Vec<OsString>>> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Ok(None);
    }
    let path_var = env.get(&path_key(env)).cloned().unwrap_or_default();
    let found = if Path::new(cmd).is_file() { Some(PathBuf::from(cmd)) } else { which(cmd, &path_var) };
    let Some(path) = found else { return Ok(None) };
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext != "cmd" && ext != "bat" {
        return Ok(Some(vec![path.into_os_string()]));
    }
    let text = std::fs::read(&path).map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
    let shim = regex::Regex::new(r#"(?i)"%dp0%\\([^"]+?\.(?:js|cjs|mjs))""#).expect("regex");
    if let Some(m) = shim.captures(&text) {
        let dir = path.parent().unwrap_or(Path::new("."));
        let script = dir.join(&m[1]);
        let local_node = dir.join("node.exe");
        let node = if local_node.exists() { Some(local_node) } else { which("node", &path_var) };
        if let Some(node) = node
            && script.exists()
        {
            return Ok(Some(vec![node.into_os_string(), script.into_os_string()]));
        }
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Err(err("session.agent.cmdShim", vec![("name", name)]))
}

/// Agentes para elegir al crear una sesión (el comando propio solo si está configurado).
pub fn available(custom: &CustomAgent, env: &BTreeMap<String, String>) -> Vec<Available> {
    AGENTS
        .iter()
        .filter(|a| a.id != CUSTOM || custom.is_set())
        .map(|a| {
            // Sin etiqueta, la UI muestra la del agente («Comando propio»).
            let (label, cmd) = if a.id == CUSTOM {
                (custom.label.clone(), custom.command.as_str())
            } else {
                (String::new(), a.exe)
            };
            let found = matches!(resolve_exe(cmd, env), Ok(Some(_)));
            Available { id: a.id, label, found, hooks: a.hooks, fork: a.fork }
        })
        .collect()
}

// ---------- línea de comando ----------

/// Divide una plantilla como lo hace `shlex.split(posix=False)` de la v1: espacios separan, las
/// comillas agrupan y se sacan si envuelven todo el argumento.
fn split_template(template: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in template.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                cur.push(c);
            }
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
                cur.push(c);
            }
            None if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            None => {
                started = true;
                cur.push(c);
            }
        }
    }
    if started {
        out.push(cur);
    }
    out.into_iter()
        .map(|t| {
            let b = t.as_bytes();
            if b.len() > 1 && b[0] == b[b.len() - 1] && (b[0] == b'"' || b[0] == b'\'') {
                t[1..t.len() - 1].to_string()
            } else {
                t
            }
        })
        .collect()
}

/// Plantilla de argumentos con `{placeholders}`. `{prompt}` vacío se quita junto con el flag que lo
/// precede (`-i {prompt}`).
pub fn custom_args(template: &str, values: &BTreeMap<&str, String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for tok in split_template(template) {
        if tok == "{prompt}" && values.get("prompt").is_none_or(String::is_empty) {
            if out.last().is_some_and(|l| l.starts_with('-')) {
                out.pop();
            }
            continue;
        }
        let mut tok = tok;
        for key in PLACEHOLDERS {
            tok = tok.replace(&format!("{{{key}}}"), values.get(key).map_or("", String::as_str));
        }
        out.push(tok);
    }
    out
}

/// Archivos que recibe una sesión.
pub struct LaunchFiles<'a> {
    pub context: &'a Path,
    pub mcp_config: &'a Path,
    /// Settings de Claude (hooks y permisos); solo Claude.
    pub claude_settings: Option<&'a Path>,
}

/// Cómo arranca: nueva, retomando o bifurcando otra conversación.
pub enum Start<'a> {
    New,
    Resume,
    Fork(&'a str),
}

/// Prefijo de planificación que de verdad se usa. Si es un comando de Claude (`/go `) que no está instalado (skill o
/// comando del usuario, del proyecto o de un plugin), Claude lo rechaza y la sesión no arranca: se manda el pedido
/// solo, que ya dice qué hacer.
pub fn effective_plan_prefix(
    agent: &str,
    prefix: &str,
    cwd: &Path,
    env: &BTreeMap<String, String>,
) -> String {
    let Some(name) = prefix.trim().strip_prefix('/').and_then(|r| r.split_whitespace().next()) else {
        return prefix.to_string();
    };
    if agent != CLAUDE || claude_command_exists(name, cwd, env) {
        return prefix.to_string();
    }
    String::new()
}

/// `~/.claude` (o `CLAUDE_CONFIG_DIR`).
fn claude_home(env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let get = |k: &str| env.get(k).cloned().or_else(|| std::env::var(k).ok()).filter(|v| !v.is_empty());
    get("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| get("USERPROFILE").or_else(|| get("HOME")).map(|h| PathBuf::from(h).join(".claude")))
}

/// ¿Hay un skill o comando `name` en el usuario, en `cwd` o sus carpetas padre, o en un plugin instalado?
pub fn claude_command_exists(name: &str, cwd: &Path, env: &BTreeMap<String, String>) -> bool {
    let name = name.rsplit(':').next().unwrap_or(name);
    let has = |dir: &Path| {
        dir.join("skills").join(name).join("SKILL.md").is_file()
            || dir.join("commands").join(format!("{name}.md")).is_file()
    };
    let home = claude_home(env);
    if home.as_deref().is_some_and(has) || cwd.ancestors().any(|d| has(&d.join(".claude"))) {
        return true;
    }
    // Plugins: `<home>/plugins/**/{skills/<name>/SKILL.md, commands/<name>.md}`, con profundidad acotada.
    let Some(plugins) = home.map(|h| h.join("plugins")) else { return false };
    let mut stack = vec![(plugins, 0u8)];
    while let Some((dir, depth)) = stack.pop() {
        if has(&dir) {
            return true;
        }
        if depth >= 6 {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
                stack.push((e.path(), depth + 1));
            }
        }
    }
    false
}

/// `(argv, variables extra)` para lanzar la sesión con su agente.
pub fn build_command(
    sess: &Session,
    files: &LaunchFiles,
    start: Start,
    prompt: Option<&str>,
    plan_prefix: &str,
    custom: &CustomAgent,
    env: &BTreeMap<String, String>,
) -> Result<(Vec<OsString>, BTreeMap<String, String>)> {
    let def = agent(&sess.agent)
        .ok_or_else(|| err("session.agent.unknown", vec![("agent", sess.agent.clone())]))?;
    let prompt = prompt.filter(|p| !p.is_empty());
    let ctx = files.context.display().to_string();
    let mcp = files.mcp_config.display().to_string();
    let mut extra = BTreeMap::new();

    if def.id == CUSTOM {
        let exe = resolve_exe(&custom.command, env)?
            .ok_or_else(|| err("session.agent.customMissing", vec![("command", custom.command.clone())]))?;
        let values: BTreeMap<&str, String> = [
            ("session_id", sess.id.clone()),
            ("name", sess.name.clone()),
            ("prompt", prompt.unwrap_or_default().to_string()),
            ("context_file", ctx.clone()),
            ("context_dir", files.context.parent().map(|p| p.display().to_string()).unwrap_or_default()),
            ("mcp_config", mcp.clone()),
            ("cwd", sess.cwd.clone()),
        ]
        .into_iter()
        .collect();
        let resume = matches!(start, Start::Resume);
        let tpl =
            if resume && !custom.resume_args.trim().is_empty() { &custom.resume_args } else { &custom.args };
        let mut argv = exe;
        argv.extend(custom_args(tpl, &values).into_iter().map(OsString::from));
        extra.insert("WORKBOARD_CONTEXT_FILE".into(), ctx);
        extra.insert("WORKBOARD_MCP_CONFIG".into(), mcp);
        return Ok((argv, extra));
    }

    let mut argv = resolve_exe(def.exe, env)?
        .ok_or_else(|| err("session.agent.notFound", vec![("command", def.exe.to_string())]))?;
    let mut push = |args: &[&str]| argv.extend(args.iter().map(OsString::from));
    let prompt = prompt
        .map(|p| if sess.plan && !plan_prefix.is_empty() { format!("{plan_prefix}{p}") } else { p.into() });

    if def.id == CLAUDE {
        match start {
            Start::Fork(from) => {
                push(&["--resume", from, "--fork-session", "--session-id", &sess.id, "-n", &sess.name])
            }
            Start::Resume => push(&["--resume", &sess.id]),
            Start::New => push(&["--session-id", &sess.id, "-n", &sess.name]),
        }
        push(&["--append-system-prompt-file", &ctx, "--mcp-config", &mcp]);
        if let Some(settings) = files.claude_settings {
            push(&["--settings", &settings.display().to_string()]);
        }
        if sess.mode == "plan" || sess.mode == "acceptEdits" {
            push(&["--permission-mode", &sess.mode]);
        }
        if let Some(model) = &sess.model {
            push(&["--model", model]);
        }
        if let Some(p) = &prompt {
            push(&[p]); // primer mensaje; al final porque --mcp-config acepta varios valores
        }
        return Ok((argv, extra));
    }

    // GitHub Copilot CLI: --session-id crea la sesión con ese uuid o la retoma. Las instrucciones del MCP
    // solo entran con --allow-all-mcp-server-instructions; el contexto, como AGENTS.md de una carpeta
    // extra de instrucciones.
    push(&["--session-id", &sess.id]);
    if !matches!(start, Start::Resume) {
        push(&["-n", &sess.name]);
    }
    push(&[
        "--additional-mcp-config",
        &format!("@{mcp}"),
        "--allow-all-mcp-server-instructions",
        "--allow-tool",
        "workboard",
    ]);
    if let Some(model) = &sess.model {
        push(&["--model", model]);
    }
    match sess.mode.as_str() {
        "plan" => push(&["--plan"]),
        "acceptEdits" => push(&["--allow-tool", "write"]),
        _ => {}
    }
    if let Some(p) = &prompt {
        push(&["-i", p]);
    }
    let ctx_dir = files.context.parent().map(|p| p.display().to_string()).unwrap_or_default();
    let mut dirs = vec![ctx_dir];
    if let Some(prev) = env.get("COPILOT_CUSTOM_INSTRUCTIONS_DIRS") {
        dirs.extend(prev.split(',').filter(|d| !d.is_empty()).map(String::from));
    }
    extra.insert("COPILOT_CUSTOM_INSTRUCTIONS_DIRS".into(), dirs.join(","));
    Ok((argv, extra))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(prompt: &str) -> BTreeMap<&'static str, String> {
        [("session_id", "S".to_string()), ("mcp_config", "C:/m.json".into()), ("prompt", prompt.into())]
            .into_iter()
            .collect()
    }

    #[test]
    fn plan_prefix_only_when_the_command_exists() {
        let d = std::env::temp_dir().join(format!("wb-sess-prefix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let home = d.join("claude");
        let cwd = d.join("work/item");
        std::fs::create_dir_all(&cwd).unwrap();
        let env: BTreeMap<String, String> =
            [("CLAUDE_CONFIG_DIR".to_string(), home.display().to_string())].into_iter().collect();
        assert_eq!(effective_plan_prefix(CLAUDE, "/go ", &cwd, &env), "");
        assert_eq!(effective_plan_prefix(CLAUDE, "Planeá: ", &cwd, &env), "Planeá: ");
        assert_eq!(effective_plan_prefix(COPILOT, "/go ", &cwd, &env), "/go ");
        let plugin = home.join("plugins/cache/mk/tools/1.0.0/skills/go");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("SKILL.md"), "").unwrap();
        assert_eq!(effective_plan_prefix(CLAUDE, "/go ", &cwd, &env), "/go ");
        std::fs::remove_dir_all(home.join("plugins")).unwrap();
        std::fs::create_dir_all(d.join("work/.claude/commands")).unwrap();
        std::fs::write(d.join("work/.claude/commands/go.md"), "").unwrap();
        assert_eq!(effective_plan_prefix(CLAUDE, "/go ", &cwd, &env), "/go ");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn template_placeholders_and_empty_prompt() {
        let tpl = r#"--mcp "{mcp_config}" --id {session_id} -i {prompt}"#;
        assert_eq!(custom_args(tpl, &values("")), ["--mcp", "C:/m.json", "--id", "S"]);
        assert_eq!(
            custom_args(tpl, &values("hola che")),
            ["--mcp", "C:/m.json", "--id", "S", "-i", "hola che"]
        );
        assert_eq!(split_template(r#"a "b c" 'd'"#), ["a", "b c", "d"]);
    }

    fn fake_env(dir: &Path) -> BTreeMap<String, String> {
        [("PATH".to_string(), dir.display().to_string())].into_iter().collect()
    }

    #[test]
    fn claude_command_line() {
        let dir = std::env::temp_dir().join(format!("wb-agents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
        std::fs::write(dir.join(exe), b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join(exe), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let sess = Session {
            id: "id1".into(),
            name: "WI 1".into(),
            agent: CLAUDE.into(),
            mode: "plan".into(),
            plan: true,
            model: Some("opus".into()),
            ..Default::default()
        };
        let files = LaunchFiles {
            context: Path::new("ctx/AGENTS.md"),
            mcp_config: Path::new("mcp.json"),
            claude_settings: Some(Path::new("s.json")),
        };
        let env = fake_env(&dir);
        let (argv, _) = build_command(
            &sess,
            &files,
            Start::New,
            Some("hacé el plan"),
            "/go ",
            &CustomAgent::default(),
            &env,
        )
        .unwrap();
        let argv: Vec<String> = argv.iter().skip(1).map(|a| a.to_string_lossy().to_string()).collect();
        assert_eq!(
            argv,
            [
                "--session-id",
                "id1",
                "-n",
                "WI 1",
                "--append-system-prompt-file",
                "ctx/AGENTS.md",
                "--mcp-config",
                "mcp.json",
                "--settings",
                "s.json",
                "--permission-mode",
                "plan",
                "--model",
                "opus",
                "/go hacé el plan"
            ]
        );
        let (fork, _) =
            build_command(&sess, &files, Start::Fork("p1"), None, "", &CustomAgent::default(), &env).unwrap();
        assert_eq!(fork[1..5], ["--resume", "p1", "--fork-session", "--session-id"].map(OsString::from));
        let missing = build_command(
            &Session { agent: COPILOT.into(), ..sess.clone() },
            &files,
            Start::New,
            None,
            "",
            &CustomAgent::default(),
            &env,
        );
        assert_eq!(missing.unwrap_err().code, "session.agent.notFound");
        let avail = available(&CustomAgent::default(), &env);
        assert!(avail.iter().any(|a| a.id == CLAUDE && a.found));
        assert!(avail.iter().all(|a| a.id != CUSTOM));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
