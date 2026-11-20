//! Lo que depende del sistema operativo, en un solo lugar: carpeta de configuración, abrir archivos,
//! carpetas y URLs, terminar un árbol de procesos, buscar ejecutables, correr scripts y el almacén de
//! credenciales (`secrets`). El resto del código no usa `cfg(windows)`.

pub mod dpapi;
pub mod secrets;

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Nombre de la carpeta de configuración. Distinto del de la v1 (`Workboard`) para que las dos
/// versiones convivan sin pisarse; se reemplaza cuando el producto tenga nombre definitivo.
pub const APP_DIR_NAME: &str = "Workboard2";

/// Variable que cambia la carpeta de configuración (pruebas, varias instancias).
pub const CONFIG_DIR_ENV: &str = "WORKBOARD_CONFIG_DIR";

/// Carpeta de configuración: `WORKBOARD_CONFIG_DIR`, o la del sistema (`%APPDATA%`,
/// `~/Library/Application Support`, `$XDG_CONFIG_HOME`) + [`APP_DIR_NAME`].
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV).filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::config_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR_NAME)
}

/// Nombre de la carpeta de configuración de la v1 (la app Python).
pub const V1_APP_DIR_NAME: &str = "Workboard";

/// Carpeta de configuración de la v1 (`%APPDATA%\Workboard`), de donde migra `workboard migrate-v1`.
/// `None` fuera de Windows: la v1 solo corría ahí. No mira `WORKBOARD_CONFIG_DIR` (esa es la de la v2).
pub fn v1_config_dir() -> Option<PathBuf> {
    if !dpapi::SUPPORTED {
        return None;
    }
    dirs::config_dir().map(|d| d.join(V1_APP_DIR_NAME))
}

/// Comando sin ventana de consola en Windows (las apps de escritorio no deben hacer parpadear una) y sin
/// salida (`taskkill` avisa cada proceso que termina).
fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Sin ventana de consola en Windows para un comando que lanza la app (git y otras herramientas de
/// línea de comandos): no hace nada en los demás sistemas.
pub fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Proceso de fondo que después se termina con `kill_tree`: sin ventana de consola en Windows y líder de
/// su grupo de procesos en Unix.
pub fn background(cmd: &mut Command) {
    hide_console(cmd);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
}

/// Abre un archivo o carpeta con la aplicación predeterminada del sistema.
pub fn open_path(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, path.display().to_string()));
    }
    let mut cmd = if cfg!(windows) {
        let mut c = command("cmd");
        // `start` toma el primer argumento entre comillas como título de la ventana: va uno vacío.
        c.args(["/C", "start", ""]).arg(path);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = command("open");
        c.arg(path);
        c
    } else {
        let mut c = command("xdg-open");
        c.arg(path);
        c
    };
    cmd.spawn().map(drop)
}

/// Abre una URL `https://` en el navegador del sistema. Otros esquemas se rechazan.
pub fn open_url(url: &str) -> io::Result<()> {
    let ok = url.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty()
            && !rest.chars().any(|c| c.is_whitespace() || c.is_control() || "\"'<>^`|".contains(c))
    });
    if !ok {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not an https URL"));
    }
    let mut cmd = if cfg!(windows) {
        // Sin `cmd /C start`: el `&` de una URL se interpretaría como otro comando.
        let mut c = command("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = command("open");
        c.arg(url);
        c
    } else {
        let mut c = command("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn().map(drop)
}

/// Manda un archivo a la papelera del sistema (Papelera de Windows, Trash de macOS, la de freedesktop
/// en Linux), de donde se puede recuperar. Nunca borra en duro.
pub fn move_to_trash(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, path.display().to_string()));
    }
    trash::delete(path).map_err(|e| io::Error::other(e.to_string()))
}

/// Muestra el archivo seleccionado en el explorador del sistema (o abre la carpeta, en Linux).
pub fn reveal_path(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, path.display().to_string()));
    }
    if cfg!(windows) {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path);
        command("explorer").arg(arg).spawn().map(drop)
    } else if cfg!(target_os = "macos") {
        command("open").arg("-R").arg(path).spawn().map(drop)
    } else {
        open_path(if path.is_dir() { path } else { path.parent().unwrap_or(path) })
    }
}

/// Abre una carpeta en VS Code (`code` del `PATH`). `Ok(false)` si VS Code no está instalado.
pub fn open_in_editor(path: &Path) -> io::Result<bool> {
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, path.display().to_string()));
    }
    let Some(exe) = which_env(&["code"]) else { return Ok(false) };
    let mut cmd = Command::new(exe);
    cmd.arg(path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    hide_console(&mut cmd);
    cmd.spawn().map(|_| true)
}

/// Termina un proceso y todos sus hijos (las sesiones de agentes lanzan sub-procesos).
///
/// En Unix el proceso tiene que haberse lanzado como líder de su grupo (`process_group(0)`): se manda
/// la señal al grupo entero.
pub fn kill_tree(pid: u32) -> io::Result<()> {
    let status = if cfg!(windows) {
        command("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).status()?
    } else {
        let group = format!("-{pid}");
        let status = command("kill").args(["-TERM", "--", &group]).status()?;
        if status.success() { status } else { command("kill").args(["-TERM", &pid.to_string()]).status()? }
    };
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("could not terminate process {pid}")))
    }
}

/// Separador de rutas del `PATH`.
pub const PATH_SEP: char = if cfg!(windows) { ';' } else { ':' };

/// Busca un ejecutable en `path` (con las extensiones de `PATHEXT` en Windows).
pub fn which(cmd: &str, path: &str) -> Option<PathBuf> {
    // En Windows un nombre sin extensión se prueba con las de PATHEXT (un archivo sin extensión, como el
    // script de bash que deja npm, no se puede ejecutar).
    let exts: Vec<String> = if cfg!(windows) && Path::new(cmd).extension().is_none() {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        pathext.split(';').filter(|e| !e.is_empty()).map(str::to_lowercase).collect()
    } else {
        vec![String::new()]
    };
    path.split(PATH_SEP)
        .filter(|d| !d.is_empty())
        .find_map(|dir| exts.iter().map(|e| Path::new(dir).join(format!("{cmd}{e}"))).find(|p| p.is_file()))
}

fn which_env(cmds: &[&str]) -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    cmds.iter().find_map(|c| which(c, &path))
}

/// Línea de comando para correr un script con su intérprete (`python`, `powershell`, `bash`, `node`,
/// `cmd`). `None` si el intérprete no está instalado o no existe en este sistema.
pub fn script_command(interpreter: &str, script: &Path) -> Option<Vec<OsString>> {
    let s = script.as_os_str().to_os_string();
    let argv = |exe: PathBuf, pre: &[&str]| {
        let mut v = vec![exe.into_os_string()];
        v.extend(pre.iter().map(OsString::from));
        v.push(s.clone());
        v
    };
    match interpreter {
        // `-I`: modo aislado (no importa módulos de la carpeta de trabajo ni del usuario).
        "python" => {
            let cands: &[&str] = if cfg!(windows) { &["py", "python"] } else { &["python3", "python"] };
            which_env(cands).map(|e| argv(e, &["-I"]))
        }
        "powershell" => which_env(&["pwsh", "powershell"])
            .map(|e| argv(e, &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])),
        "bash" => which_env(&["bash"]).map(|e| argv(e, &[])),
        "node" => which_env(&["node"]).map(|e| argv(e, &[])),
        "cmd" if cfg!(windows) => {
            let exe = std::env::var_os("COMSPEC").map_or_else(|| PathBuf::from("cmd.exe"), PathBuf::from);
            Some(argv(exe, &["/c"]))
        }
        _ => None,
    }
}

/// Resultado de [`run_captured`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// Código de salida (`None` si se cortó por tiempo o terminó por una señal).
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

/// Corre un programa sin ventana y sin entrada, captura su salida y lo corta (con sus hijos) si pasa
/// `timeout`.
pub fn run_captured(argv: &[OsString], cwd: &Path, timeout: Duration) -> io::Result<Captured> {
    let (exe, args) = argv.split_first().ok_or_else(|| io::Error::other("empty command"))?;
    let mut cmd = Command::new(exe);
    cmd.args(args).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()?;
    let drain = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = drain(child.stdout.take().map(|r| Box::new(r) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|r| Box::new(r) as Box<dyn Read + Send>));
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = kill_tree(child.id());
            let _ = child.kill();
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    Ok(Captured { code: if timed_out { None } else { status.code() }, stdout, stderr, timed_out })
}

/// Carpeta de descargas del usuario (para exportar); si no hay, la carpeta personal.
pub fn downloads_dir() -> PathBuf {
    dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_has_an_app_folder() {
        // No se toca la variable de entorno acá (los tests corren en paralelo): solo el caso sin ella.
        if std::env::var_os(CONFIG_DIR_ENV).is_none() {
            assert!(config_dir().ends_with(APP_DIR_NAME));
        }
    }

    #[test]
    fn only_https_urls_are_opened() {
        for bad in ["http://x.com", "file:///c:/x", "https://", "https://a b", "javascript:alert(1)"] {
            assert_eq!(open_url(bad).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{bad}");
        }
    }

    #[test]
    fn opening_a_missing_path_fails_without_spawning() {
        let missing = Path::new("definitely/not/here.txt");
        assert_eq!(open_path(missing).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(reveal_path(missing).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn kill_tree_terminates_a_process() {
        #[cfg(windows)]
        let mut child = Command::new("cmd").args(["/C", "ping -n 30 127.0.0.1 > NUL"]).spawn().unwrap();
        #[cfg(unix)]
        let mut child = {
            use std::os::unix::process::CommandExt;
            Command::new("sleep").arg("30").process_group(0).spawn().unwrap()
        };
        kill_tree(child.id()).unwrap();
        let status = child.wait().unwrap();
        assert!(!status.success());
    }

    #[test]
    fn runs_a_command_and_captures_its_output() {
        let dir = std::env::temp_dir();
        #[cfg(windows)]
        let argv: Vec<OsString> =
            ["cmd", "/C", "echo hola& echo error 1>&2& exit 3"].map(OsString::from).to_vec();
        #[cfg(unix)]
        let argv: Vec<OsString> =
            ["sh", "-c", "echo hola; echo error >&2; exit 3"].map(OsString::from).to_vec();
        let out = run_captured(&argv, &dir, Duration::from_secs(30)).unwrap();
        assert_eq!(out.code, Some(3));
        assert!(String::from_utf8_lossy(&out.stdout).contains("hola"));
        assert!(String::from_utf8_lossy(&out.stderr).contains("error"));
        assert!(!out.timed_out);
    }

    #[test]
    fn a_slow_command_is_cut() {
        #[cfg(windows)]
        let argv: Vec<OsString> = ["cmd", "/C", "ping -n 30 127.0.0.1 > NUL"].map(OsString::from).to_vec();
        #[cfg(unix)]
        let argv: Vec<OsString> = ["sleep", "30"].map(OsString::from).to_vec();
        let start = Instant::now();
        let out = run_captured(&argv, &std::env::temp_dir(), Duration::from_millis(500)).unwrap();
        assert!(out.timed_out && out.code.is_none());
        assert!(start.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn unknown_interpreters_have_no_command() {
        assert!(script_command("cobol", Path::new("x.cbl")).is_none());
    }
}
