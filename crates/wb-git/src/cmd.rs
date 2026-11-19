//! Cómo se llama a git: solo subcomandos de lectura (la lista está acá y se controla en cada llamada),
//! siempre con `--no-optional-locks` (ni `status` toca el index), sin ventana de consola y con un tope de
//! tiempo.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;

use crate::GitError;

/// Los únicos subcomandos que se lanzan. Nada de `fetch`, `checkout` ni nada que escriba.
pub const READ_ONLY: &[&str] =
    &["for-each-ref", "rev-list", "cherry", "merge-base", "diff", "diff-tree", "log", "patch-id", "status"];

pub const TIMEOUT: Duration = Duration::from_secs(60);

/// Git for Windows: `cmd/git.exe` es un lanzador que abre otro proceso (`mingw64/bin/git.exe`). Llamar
/// directo al segundo ahorra un proceso por comando.
fn git_exe() -> &'static Path {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let name = format!("git{}", std::env::consts::EXE_SUFFIX);
        let found = std::env::var_os("PATH")
            .and_then(|p| std::env::split_paths(&p).map(|d| d.join(&name)).find(|f| f.is_file()));
        let Some(exe) = found else { return PathBuf::from("git") };
        let parent = exe.parent().unwrap_or(Path::new(""));
        if parent.file_name().is_some_and(|n| n.eq_ignore_ascii_case("cmd"))
            && let Some(root) = parent.parent()
        {
            let direct = root.join("mingw64").join("bin").join(&name);
            if direct.is_file() {
                return direct;
            }
        }
        exe
    })
}

pub(crate) struct Output {
    pub ok: bool,
    pub stdout: Vec<u8>,
}

impl Output {
    /// Salida como texto (UTF-8 con reemplazo, como la v1).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

fn repo_name(repo: &Path) -> String {
    repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| repo.display().to_string())
}

/// Corre `git <args>` en `repo`. Con `check`, un código de salida distinto de 0 es error.
pub(crate) fn git(repo: &Path, args: &[&str], stdin: Option<&[u8]>, check: bool) -> Result<Output, GitError> {
    let sub = args.first().copied().unwrap_or("");
    if !READ_ONLY.contains(&sub) {
        return Err(GitError::new("git.notReadOnly", &[("cmd", sub)]));
    }
    let name = repo_name(repo);
    let mut cmd = Command::new(git_exe());
    cmd.arg("--no-optional-locks")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    wb_platform::hide_console(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            GitError::new("git.notInstalled", &[])
        } else {
            GitError::new("git.spawn", &[("repo", &name), ("message", &e.to_string())])
        }
    })?;
    let pid = child.id();
    let input = stdin.map(<[u8]>::to_vec);
    let mut pipe_in = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let (Some(p), Some(bytes)) = (pipe_in.as_mut(), input) {
            let _ = p.write_all(&bytes);
        }
        drop(pipe_in);
    });
    let mut pipe_out = child.stdout.take().expect("stdout piped");
    let mut pipe_err = child.stderr.take().expect("stderr piped");
    let reader = std::thread::spawn(move || {
        let mut v = vec![];
        let _ = pipe_out.read_to_end(&mut v);
        v
    });
    let reader_err = std::thread::spawn(move || {
        let mut v = vec![];
        let _ = pipe_err.read_to_end(&mut v);
        v
    });
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait());
    });
    let status = match rx.recv_timeout(TIMEOUT) {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return Err(GitError::new("git.spawn", &[("repo", &name), ("message", &e.to_string())]));
        }
        Err(_) => {
            let _ = wb_platform::kill_tree(pid);
            let what = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
            return Err(GitError::new(
                "git.timeout",
                &[("cmd", &what), ("repo", &name), ("seconds", &TIMEOUT.as_secs().to_string())],
            ));
        }
    };
    let _ = writer.join();
    let stdout = reader.join().unwrap_or_default();
    let stderr = reader_err.join().unwrap_or_default();
    if check && !status.success() {
        let what = args.iter().take(3).copied().collect::<Vec<_>>().join(" ");
        let err = String::from_utf8_lossy(&stderr);
        let err: String = err.trim().chars().take(300).collect();
        return Err(GitError::new("git.failed", &[("cmd", &what), ("repo", &name), ("message", &err)]));
    }
    Ok(Output { ok: status.success(), stdout })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_read_only_subcommands_run() {
        for sub in ["fetch", "checkout", "commit", "push", "reset", "switch", "cherry-pick", "merge", ""] {
            let err = git(Path::new("."), &[sub], None, true).err().expect("refused");
            assert_eq!(err.code, "git.notReadOnly", "{sub}");
        }
    }
}
