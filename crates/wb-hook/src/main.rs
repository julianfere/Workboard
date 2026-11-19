//! Hook de los CLI de agentes para las sesiones que lanza Workboard: le avisa el evento a la app.
//!
//!     wb-hook <Evento>      (lo configura la app en los settings de cada sesión)
//!
//! Lee el JSON del evento por stdin y lo postea a `127.0.0.1:$WORKBOARD_HOOK_PORT/hook/<Evento>` con
//! `$WORKBOARD_HOOK_TOKEN`. Imprime lo que contesta la app (el CLI lo suma al contexto del turno o, en
//! `Stop`, lo toma como decisión). Nunca falla: sin app, sin token o con error, sale 0 en silencio.
//! Sin dependencias: lo que importa es el tiempo de arranque (se corre en cada herramienta).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

fn main() {
    let event = std::env::args().nth(1).unwrap_or_else(|| "unknown".into());
    let mut body = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut body);
    if let Some(out) = post(&event, &body) {
        let _ = std::io::stdout().write_all(&out);
    }
}

fn post(event: &str, body: &[u8]) -> Option<Vec<u8>> {
    let port: u16 = std::env::var("WORKBOARD_HOOK_PORT").ok()?.parse().ok()?;
    let token = std::env::var("WORKBOARD_HOOK_TOKEN").ok().filter(|t| !t.is_empty())?;
    let session = std::env::var("WORKBOARD_SESSION").unwrap_or_default();
    if !event.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(4))).ok()?;
    let head = format!(
        "POST /hook/{event} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Workboard-Token: {token}\r\n\
         X-Workboard-Session: {session}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).ok()?;
    s.write_all(body).ok()?;
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).ok()?;
    let split = resp.windows(4).position(|w| w == b"\r\n\r\n")?;
    let status_ok = resp.starts_with(b"HTTP/1.1 200") || resp.starts_with(b"HTTP/1.0 200");
    let out = resp.split_off(split + 4);
    (status_ok && !out.is_empty() && !is_chunked(&resp[..split])).then_some(out)
}

/// La app responde con Content-Length; si algún día llega chunked, mejor no imprimir basura.
fn is_chunked(head: &[u8]) -> bool {
    String::from_utf8_lossy(head).to_ascii_lowercase().contains("transfer-encoding: chunked")
}
