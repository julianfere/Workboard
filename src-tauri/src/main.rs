//! Workboard: app de escritorio. Toda la API pasa por un único comando, `api(cmd, args)`, que despacha
//! `wb_api::Api` (lo mismo que sirve `wb-devserver` por HTTP en desarrollo).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::Value;
use tauri::State;
use wb_api::{Api, ApiResult};

#[tauri::command]
fn api(state: State<'_, Api>, cmd: String, args: Option<Value>) -> ApiResult<Value> {
    state.dispatch(&cmd, &args.unwrap_or(Value::Null))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Api::new(wb_platform::config_dir()))
        .invoke_handler(tauri::generate_handler![api])
        .run(tauri::generate_context!())
        .expect("error while running Workboard");
}
