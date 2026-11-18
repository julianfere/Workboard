//! Spike del hook del CLI: lee el JSON del evento por stdin, lo postea a la app (HTTP local con token)
//! y sale 0 en silencio. Sin dependencias: lo que importa es el tiempo de arranque.
//!
//!     wb-hook <Evento>      (WORKBOARD_HOOK_URL=http://127.0.0.1:PORT, WORKBOARD_HOOK_TOKEN=...)
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn main() {
    let event = std::env::args().nth(1).unwrap_or_default();
    let mut body = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut body);
    let (Ok(url), Ok(token)) = (std::env::var("WORKBOARD_HOOK_URL"), std::env::var("WORKBOARD_HOOK_TOKEN")) else {
        return; // sin app escuchando: no molesta al CLI
    };
    let host = url.trim_start_matches("http://").trim_end_matches('/');
    if let Ok(mut s) = TcpStream::connect(host) {
        let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
        let head = format!(
            "POST /hook/{event} HTTP/1.1\r\nHost: {host}\r\nX-Workboard-Token: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = s.write_all(head.as_bytes());
        let _ = s.write_all(&body);
    }
}
