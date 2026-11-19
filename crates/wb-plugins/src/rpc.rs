//! Una llamada a un plugin: lo lanza, manda `initialize` y el método, atiende los pedidos `host/…` que
//! hace mientras tanto y lo cierra. Un proceso por paso: el plugin no guarda nada entre llamadas.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value as Json, json};

use crate::manifest::{PROTOCOL, Plugin};

/// Lo que atiende los pedidos del plugin (la app).
pub trait Host {
    /// `Err` lleva un código (`plugin.denied`, `core.…`) que vuelve al plugin como error JSON-RPC.
    fn request(&mut self, method: &str, params: &Json) -> Result<Json, String>;
}

/// Por qué falló una llamada. `code` es una clave de `locales/*.json` (`plugin.error.*`).
#[derive(Debug, Clone, PartialEq)]
pub struct PluginError {
    pub code: &'static str,
    pub detail: String,
}

impl PluginError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self { code, detail: detail.into() }
    }
}

/// Lo que dejó el plugin en stderr (para el log), recortado.
const STDERR_MAX: usize = 64 * 1024;
/// Una línea de stdout más larga que esto es un error de protocolo.
const LINE_MAX: usize = 16 * 1024 * 1024;

struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<Result<String, String>>,
    stderr: std::thread::JoinHandle<String>,
}

impl Running {
    fn send(&mut self, msg: &Json) -> Result<(), PluginError> {
        let stdin = self.stdin.as_mut().ok_or_else(|| PluginError::new("plugin.error.exited", ""))?;
        let mut line = serde_json::to_string(msg)
            .map_err(|e| PluginError::new("plugin.error.protocol", e.to_string()))?;
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.flush())
            .map_err(|e| PluginError::new("plugin.error.exited", e.to_string()))
    }

    /// Termina el proceso: cierra stdin, le da 2 s para salir y si no, lo mata con sus hijos.
    fn finish(mut self) -> String {
        self.stdin = None;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = wb_platform::kill_tree(self.child.id());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.stderr.join().unwrap_or_default()
    }
}

fn spawn(plugin: &Plugin) -> Result<Running, PluginError> {
    let argv = plugin.argv().map_err(|e| PluginError::new("plugin.error.command", e))?;
    let (exe, args) = argv.split_first().ok_or_else(|| PluginError::new("plugin.error.command", "empty"))?;
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .current_dir(&plugin.dir)
        .env("WORKBOARD_PLUGIN_PROTOCOL", PROTOCOL.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    wb_platform::background(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| PluginError::new("plugin.error.spawn", e.to_string()))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().ok_or_else(|| PluginError::new("plugin.error.spawn", "no stdout"))?;
    let stderr = child.stderr.take();
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        let mut r = BufReader::new(stdout);
        loop {
            let mut buf = Vec::new();
            match r.by_ref().take(LINE_MAX as u64 + 1).read_until(b'\n', &mut buf) {
                Ok(0) => break,
                Ok(_) if buf.len() > LINE_MAX => {
                    let _ = tx.send(Err("line too long".into()));
                    break;
                }
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buf).trim().to_string();
                    if !text.is_empty() && tx.send(Ok(text)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    break;
                }
            }
        }
    });
    let stderr = std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(e) = stderr {
            let mut e = e;
            let mut chunk = [0u8; 8192];
            while let Ok(n) = e.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if out.len() < STDERR_MAX {
                    out.extend_from_slice(&chunk[..n.min(STDERR_MAX - out.len())]);
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    });
    Ok(Running { child, stdin, lines, stderr })
}

/// Espera la respuesta a `id` atendiendo los pedidos del plugin.
fn await_reply(
    run: &mut Running,
    id: u64,
    deadline: Instant,
    host: &mut dyn Host,
) -> Result<Json, PluginError> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(PluginError::new("plugin.error.timeout", ""));
        }
        let line = match run.lines.recv_timeout(left) {
            Ok(Ok(l)) => l,
            Ok(Err(e)) => return Err(PluginError::new("plugin.error.protocol", e)),
            Err(mpsc::RecvTimeoutError::Timeout) => return Err(PluginError::new("plugin.error.timeout", "")),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(PluginError::new("plugin.error.exited", ""));
            }
        };
        let msg: Json = serde_json::from_str(&line).map_err(|e| {
            PluginError::new(
                "plugin.error.protocol",
                format!("{e}: {}", line.chars().take(200).collect::<String>()),
            )
        })?;
        if let Some(method) = msg["method"].as_str() {
            // Un pedido del plugin (sin id es una notificación: no se contesta).
            if msg.get("id").is_none_or(Json::is_null) {
                continue;
            }
            let reply = match host.request(method, &msg["params"]) {
                Ok(result) => json!({"jsonrpc": "2.0", "id": msg["id"], "result": result}),
                Err(code) => {
                    json!({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32000, "message": code}})
                }
            };
            run.send(&reply)?;
            continue;
        }
        if msg["id"].as_u64() != Some(id) {
            return Err(PluginError::new("plugin.error.protocol", format!("unexpected id {}", msg["id"])));
        }
        if let Some(err) = msg.get("error").filter(|e| !e.is_null()) {
            let text = err["message"].as_str().map_or_else(|| err.to_string(), String::from);
            return Err(PluginError::new("plugin.error.failed", text.chars().take(500).collect::<String>()));
        }
        return Ok(msg.get("result").cloned().unwrap_or(Json::Null));
    }
}

/// Llama a `method` del plugin. `stderr` recibe lo que escribió el plugin (para el log).
pub fn call(
    plugin: &Plugin,
    lang: &str,
    method: &str,
    params: Json,
    host: &mut dyn Host,
    stderr: &mut String,
) -> Result<Json, PluginError> {
    let m =
        plugin.manifest.as_ref().ok_or_else(|| PluginError::new("plugin.error.command", "no manifest"))?;
    let deadline = Instant::now() + m.timeout();
    let mut run = spawn(plugin)?;
    let init = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocol": PROTOCOL,
            "host": {"name": "workboard", "version": env!("CARGO_PKG_VERSION")},
            "lang": lang,
            "plugin": {"id": m.id, "dir": plugin.dir.display().to_string()},
        },
    });
    let out = run
        .send(&init)
        .and_then(|_| await_reply(&mut run, 1, deadline, host))
        .and_then(|_| run.send(&json!({"jsonrpc": "2.0", "id": 2, "method": method, "params": params})))
        .and_then(|_| await_reply(&mut run, 2, deadline, host));
    *stderr = run.finish();
    out
}
