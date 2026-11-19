//! Lo que depende del sistema operativo, en un solo lugar: carpeta de configuración, abrir archivos y
//! carpetas, y terminar un árbol de procesos. El resto del código no usa `cfg(windows)`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

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
}
