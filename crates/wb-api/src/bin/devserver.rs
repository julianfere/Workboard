//! Servidor de desarrollo: la misma API por HTTP, para trabajar la UI en un navegador (devtools,
//! Playwright) contra el backend real.
//!
//!     cargo run -p wb-api --bin wb-devserver -- [carpeta] [--port 8766]
//!     npm run dev     (Vite reenvía /api a este servidor)
//!
//! `POST /api/<cmd>` con los argumentos como objeto JSON y el header `X-Workboard: 1` (obliga a un
//! preflight CORS: ninguna web externa puede postear a localhost). Responde `{"ok": true, "data": …}` o
//! `{"ok": false, "error": {code, params}}`. `GET` sirve `ui/dist` si existe (UI ya compilada).
//! Usar con `WORKBOARD_CONFIG_DIR` apuntando a una carpeta temporal.
//!
//! `--fake-tracker <archivo.json>`: el tracker responde desde un archivo (`{"GET /_apis/wit/fields": {…}}`,
//! la clave es el método y el final de la ruta) y los secretos quedan en memoria. Sirve para probar la UI de
//! trackers sin red ni credenciales reales.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use tiny_http::{Header, Method, Response, Server};
use wb_api::Api;

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("valid header")
}

/// Tracker simulado: responde por método y final de la ruta; lo demás, 404.
struct FakeTracker(serde_json::Map<String, serde_json::Value>);

impl wb_tracker::Transport for FakeTracker {
    fn send(&self, req: &wb_tracker::HttpRequest) -> Result<wb_tracker::HttpResponse, String> {
        let path = req.url.split('?').next().unwrap_or_default();
        let found = self
            .0
            .iter()
            .find(|(k, _)| k.split_once(' ').is_some_and(|(m, p)| m == req.method && path.ends_with(p)));
        let (status, body) = found.map_or((404, json!({"message": "not found"})), |(_, v)| (200, v.clone()));
        Ok(wb_tracker::HttpResponse {
            status,
            content_type: "application/json".into(),
            body: body.to_string().into_bytes(),
        })
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut root, mut port, mut fake) = (None, 8766u16, None);
    while let Some(a) = args.next() {
        if a == "--port" {
            port = args.next().and_then(|p| p.parse().ok()).expect("--port <number>");
        } else if a == "--fake-tracker" {
            fake = Some(args.next().expect("--fake-tracker <file.json>"));
        } else {
            root = Some(a);
        }
    }
    let api = Api::new(wb_platform::config_dir());
    if let Some(file) = fake {
        let text = std::fs::read_to_string(&file).expect("--fake-tracker: unreadable file");
        let routes: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&text).expect("--fake-tracker: invalid JSON");
        api.set_tracker_backend(
            Arc::new(wb_platform::secrets::MemorySecrets::default()),
            Arc::new(FakeTracker(routes)),
        );
    }
    if let Some(root) = root
        && let Err(e) = api.dispatch("open_root", &json!({"path": root}))
    {
        eprintln!("no se pudo abrir {root}: {e:?}");
    }
    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/dist");
    let tauri_conf: serde_json::Value =
        serde_json::from_str(include_str!("../../../../src-tauri/tauri.conf.json")).expect("tauri.conf.json");
    let csp = tauri_conf["app"]["security"]["csp"].as_str().unwrap_or_default().to_string();
    let server = Server::http(("127.0.0.1", port)).expect("port in use?");
    println!("wb-devserver: http://127.0.0.1:{port} (config: {})", wb_platform::config_dir().display());

    for mut req in server.incoming_requests() {
        let url = req.url().split('?').next().unwrap_or("/").to_string();
        let response = if *req.method() == Method::Post && url.starts_with("/api/") {
            let allowed = req.headers().iter().any(|h| h.field.equiv("X-Workboard") && h.value == "1");
            if !allowed {
                Response::from_string("forbidden").with_status_code(403)
            } else {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let args: serde_json::Value = serde_json::from_str(&body).unwrap_or(json!({}));
                let out = match api.dispatch(&url["/api/".len()..], &args) {
                    Ok(data) => json!({"ok": true, "data": data}),
                    Err(error) => json!({"ok": false, "error": error}),
                };
                Response::from_string(out.to_string())
                    .with_header(header("Content-Type", "application/json; charset=utf-8"))
            }
        } else if *req.method() == Method::Get {
            let rel =
                if url == "/" { "index.html".to_string() } else { url.trim_start_matches('/').to_string() };
            let path = dist.join(&rel);
            match (rel.contains(".."), std::fs::read(&path)) {
                (false, Ok(bytes)) => {
                    let ctype = match path.extension().and_then(|e| e.to_str()) {
                        Some("html") => "text/html; charset=utf-8",
                        Some("js") => "text/javascript; charset=utf-8",
                        Some("css") => "text/css; charset=utf-8",
                        Some("svg") => "image/svg+xml",
                        Some("xml") => "application/xml",
                        Some("json") => "application/json",
                        Some("png") => "image/png",
                        Some("jpg" | "jpeg") => "image/jpeg",
                        Some("gif") => "image/gif",
                        Some("webp") => "image/webp",
                        Some("woff2") => "font/woff2",
                        _ => "application/octet-stream",
                    };
                    let mut res = Response::from_data(bytes).with_header(header("Content-Type", ctype));
                    if ctype.starts_with("text/html") {
                        // La misma CSP que la ventana de Tauri: lo que pasa acá pasa en la app.
                        res = res.with_header(header("Content-Security-Policy", &csp));
                    }
                    res
                }
                _ => Response::from_string("not found").with_status_code(404),
            }
        } else {
            Response::from_string("not found").with_status_code(404)
        };
        let _ = req.respond(response.with_header(header("Cache-Control", "no-store")));
    }
}
