//! Workboard: app de escritorio. Toda la API pasa por un único comando, `api(cmd, args)`, que despacha
//! `wb_api::Api` (lo mismo que sirve `wb-devserver` por HTTP en desarrollo). La salida de las terminales
//! de las sesiones es un flujo: va por `session_attach`, con un canal de bytes crudos por pestaña.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::{Value, json};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Manager, State};
use wb_api::{Api, ApiResult};
use wb_sessions::PtyEvent;

#[tauri::command]
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
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Api::new(wb_platform::config_dir()))
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
