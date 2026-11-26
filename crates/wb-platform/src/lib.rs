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

/// Ejecutable de VS Code a partir del `code` que hay en `path`. En Windows `code` es un `.cmd` (que la
/// consola parte mal con rutas entre comillas): se usa el `Code.exe` de la instalación, al lado de `bin/`.
pub fn vscode_exe(path: &str) -> Option<PathBuf> {
    let found = which("code", path)?;
    let is_script =
        found.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if !is_script {
        return Some(found);
    }
    let exe = found.parent()?.parent()?.join("Code.exe");
    exe.is_file().then_some(exe)
}

/// Abre una carpeta (o archivo) en VS Code. `NotFound` si VS Code no está en el `PATH`.
pub fn open_in_vscode(target: &Path) -> io::Result<()> {
    if !target.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, target.display().to_string()));
    }
    let exe = vscode_exe(&std::env::var("PATH").unwrap_or_default())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "code"))?;
    let mut cmd = Command::new(exe);
    cmd.arg(target).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    cmd.spawn().map(drop)
}

/// ¿Es un link que se puede abrir afuera? `https://` y `http://` con algo después, o `mailto:` con una
/// dirección; sin espacios, controles ni caracteres que una consola interpretaría.
pub fn is_external_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("mailto:"));
    rest.is_some_and(|rest| {
        !rest.is_empty()
            && !rest.chars().any(|c| c.is_whitespace() || c.is_control() || "\"'<>^`|\\".contains(c))
    })
}

/// Abre una URL `https://`, `http://` o `mailto:` en el navegador (o el cliente de correo) del sistema.
/// Otros esquemas se rechazan.
pub fn open_url(url: &str) -> io::Result<()> {
    if !is_external_url(url) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not an http(s) or mailto URL"));
    }
    launch_uri(url)
}

/// URI que abre un archivo en Obsidian (`obsidian://open?path=…`), con la ruta codificada como
/// `urllib.parse.quote` de Python (la v1): solo quedan sin codificar las letras, dígitos, `-._~` y `/`.
pub fn obsidian_uri(path: &Path) -> String {
    let mut out = String::from("obsidian://open?path=");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Abre un archivo en Obsidian (si su carpeta es, o está dentro de, un vault; si no, Obsidian lo avisa).
pub fn open_in_obsidian(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, path.display().to_string()));
    }
    launch_uri(&obsidian_uri(path))
}

/// Le pasa una URI ya validada al manejador del sistema para su esquema.
fn launch_uri(url: &str) -> io::Result<()> {
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
    fn only_web_and_mail_urls_are_opened() {
        for bad in [
            "file:///c:/x",
            "https://",
            "http://",
            "mailto:",
            "https://a b",
            "http://x.com/a\"b",
            "javascript:alert(1)",
            "obsidian://open?path=x",
            "HTTPS://x.com",
        ] {
            assert!(!is_external_url(bad), "{bad}");
            assert_eq!(open_url(bad).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{bad}");
        }
        for good in ["https://x.com/a?b=1&c=2", "http://intranet/x", "mailto:someone@example.com"] {
            assert!(is_external_url(good), "{good}");
        }
    }

    #[test]
    fn obsidian_uris_quote_the_path_like_python() {
        assert_eq!(
            obsidian_uri(Path::new("C:\\Vault\\Notas/a b-ñ.md")),
            "obsidian://open?path=C%3A%5CVault%5CNotas/a%20b-%C3%B1.md"
        );
        let missing = Path::new("definitely/not/here.md");
        assert_eq!(open_in_obsidian(missing).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn opening_a_missing_path_fails_without_spawning() {
        let missing = Path::new("definitely/not/here.txt");
        assert_eq!(open_path(missing).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(reveal_path(missing).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn vscode_is_found_next_to_its_cli() {
        let dir = std::env::temp_dir().join(format!("wb-platform-code-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        assert!(vscode_exe(&bin.display().to_string()).is_none());
        if cfg!(windows) {
            std::fs::write(bin.join("code.cmd"), "@echo off").unwrap();
            // Sin `Code.exe` al lado de `bin/` no hay a qué llamar.
            assert!(vscode_exe(&bin.display().to_string()).is_none());
            std::fs::write(dir.join("Code.exe"), "").unwrap();
            assert_eq!(vscode_exe(&bin.display().to_string()), Some(dir.join("Code.exe")));
        } else {
            std::fs::write(bin.join("code"), "#!/bin/sh").unwrap();
            assert_eq!(vscode_exe(&bin.display().to_string()), Some(bin.join("code")));
        }
        assert_eq!(
            open_in_vscode(Path::new("definitely/not/here")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let _ = std::fs::remove_dir_all(&dir);
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
