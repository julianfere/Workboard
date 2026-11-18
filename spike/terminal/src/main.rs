//! Spike: ¿la TUI de un agente (Claude Code) anda bien dentro de una ventana Tauri?
//!
//! portable-pty (ConPTY en Windows) corre el comando; la salida va a xterm.js por un `Channel` de
//! Tauri (bytes crudos, sin WebSocket).
//!
//!     cargo run --release                         # claude en el directorio actual
//!     cargo run --release -- --cwd C:\ruta         # en otra carpeta
//!     cargo run --release -- -- cmd.exe            # otro comando
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager, State};

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
}

#[derive(Default)]
struct Sessions {
    next: AtomicU32,
    map: Mutex<HashMap<u32, Session>>,
}

#[derive(Serialize, Clone)]
struct LaunchConfig {
    cmd: Vec<String>,
    cwd: String,
}

fn launch_config() -> LaunchConfig {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cwd = std::env::current_dir().unwrap().display().to_string();
    let mut cmd = vec!["claude".to_string()];
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cwd" if i + 1 < args.len() => {
                cwd = args[i + 1].clone();
                i += 2;
            }
            "--" => {
                cmd = args[i + 1..].to_vec();
                break;
            }
            _ => i += 1,
        }
    }
    LaunchConfig { cmd, cwd }
}

#[tauri::command]
fn config() -> LaunchConfig {
    launch_config()
}

#[tauri::command]
fn spawn(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    cmd: Vec<String>,
    cwd: String,
    cols: u16,
    rows: u16,
    on_data: Channel<InvokeResponseBody>,
    on_exit: Channel<u32>,
) -> Result<u32, String> {
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;
    let (prog, rest) = cmd.split_first().ok_or("comando vacío")?;
    let mut builder = CommandBuilder::new(prog);
    builder.args(rest);
    builder.cwd(cwd);
    let mut child = pair.slave.spawn_command(builder).map_err(|e| e.to_string())?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let id = sessions.next.fetch_add(1, Ordering::SeqCst);
    sessions.map.lock().unwrap().insert(
        id,
        Session { master: pair.master, writer, killer: child.clone_killer(), pid: child.process_id() },
    );

    // Lector bloqueante → hilo que agrupa. ConPTY entrega de a ~65 bytes: mandar cada lectura como
    // un mensaje IPC (y un term.write) es lo que frena. Se junta lo que llega en FLUSH_MS (o hasta
    // MAX_BATCH) y va en un solo mensaje; con poca salida (tipear) la latencia sigue siendo ~0.
    const FLUSH_MS: u64 = 8;
    const MAX_BATCH: usize = 256 * 1024;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    trace("out", &buf[..n]);
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    std::thread::spawn(move || {
        use std::sync::mpsc::RecvTimeoutError;
        use std::time::{Duration, Instant};
        while let Ok(first) = rx.recv() {
            let mut batch = first;
            let deadline = Instant::now() + Duration::from_millis(FLUSH_MS);
            let mut closed = false;
            while batch.len() < MAX_BATCH {
                let left = deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(left) {
                    Ok(more) => batch.extend_from_slice(&more),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        closed = true;
                        break;
                    }
                }
            }
            if on_data.send(InvokeResponseBody::Raw(batch)).is_err() || closed {
                break;
            }
        }
    });

    // Espera al proceso. En Windows el lector de ConPTY no ve EOF hasta que se cierra la pseudo
    // consola: al salir el hijo se suelta el master (cierra la consola, vacía lo pendiente).
    std::thread::spawn(move || {
        let code = child.wait().map(|s| s.exit_code()).unwrap_or(u32::MAX);
        trace("out", format!("<<exit {code}>>").as_bytes());
        std::thread::sleep(std::time::Duration::from_millis(50));
        app.state::<Sessions>().map.lock().unwrap().remove(&id);
        let _ = on_exit.send(code);
    });
    Ok(id)
}

/// Traza de diagnóstico en %TEMP%\spike-terminal-{in,out}.log (lo que entra y sale de la pty).
fn trace(kind: &str, data: &[u8]) {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!("spike-terminal-{kind}.log"));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        let _ = writeln!(f, "[{ms}] {:?}", String::from_utf8_lossy(data));
    }
}

#[tauri::command]
fn write(sessions: State<'_, Sessions>, id: u32, data: String) -> Result<(), String> {
    trace("in", data.as_bytes());
    let mut map = sessions.map.lock().unwrap();
    let s = map.get_mut(&id).ok_or("sesión inexistente")?;
    s.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
    s.writer.flush().map_err(|e| e.to_string())
}

#[tauri::command]
fn resize(sessions: State<'_, Sessions>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    let map = sessions.map.lock().unwrap();
    let s = map.get(&id).ok_or("sesión inexistente")?;
    s.master
        .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

/// Mata el árbol del proceso (en Windows `taskkill /T`, igual que la v1).
fn kill_tree(s: &mut Session) {
    #[cfg(windows)]
    {
        if let Some(pid) = s.pid {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .status();
            return;
        }
    }
    let _ = s.killer.kill();
}

#[tauri::command]
fn kill(sessions: State<'_, Sessions>, id: u32) {
    if let Some(mut s) = sessions.map.lock().unwrap().remove(&id) {
        kill_tree(&mut s);
    }
}

fn main() {
    tauri::Builder::default()
        .manage(Sessions::default())
        .invoke_handler(tauri::generate_handler![config, spawn, write, resize, kill])
        .on_window_event(|window, event| {
            // Cerrar la ventana mata todos los árboles: nada de huérfanos.
            if let tauri::WindowEvent::Destroyed = event {
                let sessions = window.state::<Sessions>();
                for (_, mut s) in sessions.map.lock().unwrap().drain() {
                    kill_tree(&mut s);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("no se pudo abrir la ventana");
}
