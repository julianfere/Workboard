//! Throughput de la pty sin ventana. Mismo caso que el spike de la v1 (pywinpty: 15,8 s; sin pty: 0,6 s):
//!
//!     cargo run --release --bin pty-bench
//!     cargo run --release --bin pty-bench -- cmd /c dir /s C:\Windows\System32
use std::io::Read;
use std::time::Instant;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

fn main() {
    let mut cmd: Vec<String> = std::env::args().skip(1).collect();
    if cmd.is_empty() {
        cmd = ["cmd", "/c", "dir", "/s", r"C:\Windows\System32"].map(String::from).to_vec();
    }

    // Sin pty, como referencia.
    let t = Instant::now();
    let out = std::process::Command::new(&cmd[0]).args(&cmd[1..]).output().expect("no corre");
    println!("sin pty: {} bytes en {:.2} s", out.stdout.len(), t.elapsed().as_secs_f64());

    let pair = native_pty_system()
        .openpty(PtySize { rows: 40, cols: 160, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let mut b = CommandBuilder::new(&cmd[0]);
    b.args(&cmd[1..]);
    let t = Instant::now();
    let mut child = pair.slave.spawn_command(b).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let reading = std::thread::spawn(move || {
        let (mut total, mut reads) = (0usize, 0usize);
        let mut buf = vec![0u8; 64 * 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            // ConPTY arranca pidiendo la posición del cursor (ESC[6n) y no dibuja nada hasta que
            // le contestan. En la app lo contesta xterm.js; acá lo hacemos a mano.
            if buf[..n].windows(4).any(|w| w == b"\x1b[6n") {
                use std::io::Write;
                let _ = writer.write_all(b"\x1b[1;1R");
            }
            if std::env::var_os("DUMP").is_some() {
                use std::io::Write;
                let _ = std::io::stderr().write_all(&buf[..n]);
            }
            total += n;
            reads += 1;
        }
        (total, reads)
    });
    child.wait().unwrap();
    let exited = t.elapsed().as_secs_f64();
    drop(pair.master); // cierra la pseudo consola: el lector vacía lo pendiente y ve EOF
    let (total, reads) = reading.join().unwrap();
    let secs = t.elapsed().as_secs_f64();
    println!(
        "pty: {total} bytes en {secs:.2} s (proceso terminó a los {exited:.2} s), {reads} lecturas, {:.0} KB/s",
        total as f64 / 1024.0 / secs
    );
}
