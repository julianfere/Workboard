//! Un proceso en una pseudo-terminal (ConPTY en Windows). Vive aunque la pestaña se desconecte y guarda
//! la salida reciente para redibujarla al reconectar.
//!
//! La salida se agrupa en un hilo aparte (8 ms o 256 KB): ConPTY entrega de a ~65 bytes y mandar cada
//! lectura como un mensaje a la UI es lo que frena (ver `spike/RESULTS.md`).

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Salida que se guarda por terminal para reconectar una pestaña.
pub const REPLAY_MAX: usize = 512 * 1024;
const FLUSH_MS: u64 = 8;
const MAX_BATCH: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub enum PtyEvent {
    Data(Vec<u8>),
    /// El proceso terminó (código de salida si se conoce).
    Exit(Option<u32>),
}

/// Recibe la salida; devuelve `false` cuando ya no la quiere (la pestaña se cerró).
pub type Sink = Box<dyn FnMut(&PtyEvent) -> bool + Send>;

#[derive(Default)]
struct Out {
    chunks: VecDeque<Vec<u8>>,
    size: usize,
    /// Bytes recibidos desde el principio (el búfer tiene los últimos `size`).
    total: u64,
    sinks: Vec<Sink>,
    exited: Option<Option<u32>>,
}

impl Out {
    fn push(&mut self, data: Vec<u8>) {
        let ev = PtyEvent::Data(data);
        self.sinks.retain_mut(|s| s(&ev));
        let PtyEvent::Data(data) = ev else { unreachable!() };
        self.total += data.len() as u64;
        self.size += data.len();
        self.chunks.push_back(data);
        while self.size > REPLAY_MAX && self.chunks.len() > 1 {
            self.size -= self.chunks.pop_front().map_or(0, |c| c.len());
        }
    }
}

pub struct Pty {
    out: Arc<Mutex<Out>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: Option<u32>,
}

impl Pty {
    /// Lanza `argv` en `cwd` con exactamente `env`. `on_exit` corre una vez, cuando el proceso termina y
    /// ya salió toda su salida.
    pub fn spawn(
        argv: &[OsString],
        cwd: &Path,
        env: &BTreeMap<String, String>,
        cols: u16,
        rows: u16,
        on_exit: impl FnOnce(Option<u32>) + Send + 'static,
    ) -> std::io::Result<Arc<Self>> {
        let pair = native_pty_system()
            .openpty(PtySize { rows: rows.max(5), cols: cols.max(20), pixel_width: 0, pixel_height: 0 })
            .map_err(io)?;
        let (prog, rest) = argv.split_first().ok_or_else(|| std::io::Error::other("empty command"))?;
        let mut cmd = CommandBuilder::new(prog);
        cmd.args(rest);
        cmd.cwd(cwd);
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(io)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(io)?;
        let writer = pair.master.take_writer().map_err(io)?;
        let pty = Arc::new(Self {
            out: Arc::default(),
            writer: Mutex::new(Some(writer)),
            master: Arc::new(Mutex::new(Some(pair.master))),
            killer: Mutex::new(child.clone_killer()),
            pid: child.process_id(),
        });

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let out = pty.out.clone();
        std::thread::spawn(move || {
            let _done = done_tx; // se suelta al terminar: avisa que no queda salida por mandar
            while let Ok(first) = rx.recv() {
                let mut batch = first;
                let deadline = Instant::now() + Duration::from_millis(FLUSH_MS);
                while batch.len() < MAX_BATCH {
                    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(more) => batch.extend_from_slice(&more),
                        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
                    }
                }
                out.lock().expect("pty out").push(batch);
            }
        });
        let (out, master) = (pty.out.clone(), pty.master.clone());
        std::thread::spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code());
            // En Windows el lector no ve EOF hasta que se cierra la pseudo consola: se suelta el master.
            std::thread::sleep(Duration::from_millis(50));
            master.lock().expect("pty master").take();
            let _ = done_rx.recv_timeout(Duration::from_secs(2));
            {
                let mut o = out.lock().expect("pty out");
                o.exited = Some(code);
                let ev = PtyEvent::Exit(code);
                o.sinks.retain_mut(|s| s(&ev));
            }
            on_exit(code);
        });
        Ok(pty)
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn alive(&self) -> bool {
        self.out.lock().expect("pty out").exited.is_none()
    }

    /// Suscribe una pestaña: primero recibe la salida guardada (y el fin, si ya terminó).
    pub fn attach(&self, mut sink: Sink) {
        let mut o = self.out.lock().expect("pty out");
        if !o.chunks.is_empty() {
            let replay: Vec<u8> = o.chunks.iter().flatten().copied().collect();
            if !sink(&PtyEvent::Data(replay)) {
                return;
            }
        }
        if let Some(code) = o.exited {
            sink(&PtyEvent::Exit(code));
            return;
        }
        o.sinks.push(sink);
    }

    /// Salida desde el byte `offset` (para clientes que consultan, como el servidor de desarrollo):
    /// `(nuevo offset, bytes, terminó)`. Si `offset` ya salió del búfer, empieza por lo que queda.
    pub fn read_since(&self, offset: u64) -> (u64, Vec<u8>, Option<Option<u32>>) {
        let o = self.out.lock().expect("pty out");
        let start = o.total - o.size as u64;
        let skip = offset.saturating_sub(start).min(o.size as u64) as usize;
        let data: Vec<u8> = o.chunks.iter().flatten().skip(skip).copied().collect();
        (o.total, data, o.exited)
    }

    pub fn write(&self, data: &[u8]) -> std::io::Result<()> {
        if !self.alive() {
            return Ok(());
        }
        if let Some(w) = self.writer.lock().expect("pty writer").as_mut() {
            w.write_all(data)?;
            w.flush()?;
        }
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> std::io::Result<()> {
        if let Some(m) = self.master.lock().expect("pty master").as_ref() {
            m.resize(PtySize { rows: rows.max(5), cols: cols.max(20), pixel_width: 0, pixel_height: 0 })
                .map_err(io)?;
        }
        Ok(())
    }

    /// Termina el proceso y todo su árbol (solo el de este pid).
    pub fn kill(&self) {
        if !self.alive() {
            return;
        }
        let tree = self.pid.map(wb_platform::kill_tree);
        if !matches!(tree, Some(Ok(()))) {
            let _ = self.killer.lock().expect("pty killer").kill();
        }
    }
}

/// portable-pty devuelve `anyhow::Error`: se convierte por su texto.
fn io(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}
