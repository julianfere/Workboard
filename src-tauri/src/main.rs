//! Workboard: app de escritorio. Toda la API pasa por un único comando, `api(cmd, args)`, que despacha
//! `wb_api::Api` (lo mismo que sirve `wb-devserver` por HTTP en desarrollo). La salida de las terminales
//! de las sesiones es un flujo: va por `session_attach`, con un canal de bytes crudos por pestaña.
//! `workboard migrate-v1 …` no abre la ventana: importa la config de la v1 (`docs/migrate-v1.md`).
//! `workboard plugin new|check …` tampoco: crea o prueba un plugin (`docs/plugins.md`, lógica en
//! `wb_plugins::cli`).
//! `workboard <carpeta>` abre la app con esa carpeta de trabajos (como `py app.py <carpeta>` en la v1).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::{Value, json};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Manager, State};
use wb_api::{Api, ApiResult};
use wb_sessions::PtyEvent;

// `async`: fuera del hilo principal. Algunos comandos tardan (la vista general de git puede llevar
// segundos la primera vez) y no deben congelar la ventana.
#[tauri::command(async)]
fn api(state: State<'_, Api>, cmd: String, args: Option<Value>) -> ApiResult<Value> {
    state.dispatch(&cmd, &args.unwrap_or(Value::Null))
}

/// Suscribe una pestaña a la terminal de una sesión: primero llega lo guardado y después lo nuevo. El canal
/// `on_exit` recibe `{code}` cuando el proceso termina.
#[tauri::command]
fn session_attach(
    state: State<'_, Api>,
    id: String,
    on_data: Channel<InvokeResponseBody>,
    on_exit: Channel<Value>,
) -> ApiResult<()> {
    let mgr = state.session_manager()?;
    mgr.attach(
        &id,
        Box::new(move |ev| match ev {
            PtyEvent::Data(bytes) => on_data.send(InvokeResponseBody::Raw(bytes.clone())).is_ok(),
            PtyEvent::Exit(code) => {
                let _ = on_exit.send(json!({ "code": code }));
                false
            }
        }),
    )?;
    Ok(())
}

fn main() {
    // Subcomandos de consola, antes de abrir la ventana.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("migrate-v1" | "plugin")) {
        // La app de release no tiene consola propia: se engancha a la de quien la lanzó.
        wb_platform::console::attach_parent();
    }
    if args.first().map(String::as_str) == Some("migrate-v1") {
        let code = wb_api::migrate::cli(&args[1..], &mut std::io::stdout().lock());
        std::process::exit(code);
    }
    if args.first().map(String::as_str) == Some("plugin") {
        let code =
            wb_plugins::cli::cli(&args[1..], &wb_platform::config_dir(), &mut std::io::stdout().lock());
        std::process::exit(code);
    }

    let app_api = Api::new(wb_platform::config_dir());
    // Una carpeta como argumento queda abierta (y primera de los recientes). Si no sirve, la app abre con
    // la última, como siempre.
    if let Some(folder) = args.first().filter(|a| !a.starts_with('-')) {
        let _ = app_api.dispatch("open_root", &json!({ "path": folder }));
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(app_api)
        .invoke_handler(tauri::generate_handler![api, session_attach])
        .on_window_event(|window, event| {
            // Cerrar la ventana mata las terminales (nada de huérfanos); las sesiones quedan pausadas.
            if let tauri::WindowEvent::Destroyed = event {
                window.state::<Api>().shutdown();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Workboard");
}
