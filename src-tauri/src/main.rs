//! Workboard: app de escritorio. Los comandos (`#[tauri::command]`) son la API que usa la UI; cada uno
//! es un wrapper fino sobre los crates `wb-*` y devuelve `ApiResult`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod error;
mod settings;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::State;
use wb_core::Workflow;

use error::{ApiError, ApiResult};
use settings::Settings;

struct AppState {
    config_dir: PathBuf,
    settings: Mutex<Settings>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: &'static str,
    os: &'static str,
    config_dir: String,
}

#[derive(Serialize)]
struct PresetSummary {
    id: &'static str,
    name: wb_core::workflow::Labels,
}

#[tauri::command]
fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        config_dir: state.config_dir.display().to_string(),
    }
}

#[tauri::command]
fn workflow_presets() -> ApiResult<Vec<PresetSummary>> {
    Workflow::preset_ids().map(|id| Ok(PresetSummary { id, name: Workflow::preset(id)?.name })).collect()
}

#[tauri::command]
fn workflow_preset(id: String) -> ApiResult<Workflow> {
    Ok(Workflow::preset(&id)?)
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().expect("settings lock").clone()
}

#[tauri::command]
fn set_settings(state: State<'_, AppState>, settings: Settings) -> ApiResult<Settings> {
    // Un preset desconocido no se guarda: la app no podría arrancar con él.
    Workflow::preset(&settings.workflow)?;
    if let Some(lang) = &settings.lang
        && !wb_i18n::LANGS.contains(&lang.as_str())
    {
        return Err(ApiError::generic(format!("unsupported language: {lang}")));
    }
    settings::save(&settings::path_in(&state.config_dir), &settings)?;
    *state.settings.lock().expect("settings lock") = settings.clone();
    Ok(settings)
}

fn main() {
    let config_dir = wb_platform::config_dir();
    let mut loaded = settings::load(&settings::path_in(&config_dir));
    if Workflow::preset(&loaded.workflow).is_err() {
        loaded.workflow = Settings::default().workflow;
    }
    tauri::Builder::default()
        .manage(AppState { config_dir, settings: Mutex::new(loaded) })
        .invoke_handler(tauri::generate_handler![
            app_info,
            workflow_presets,
            workflow_preset,
            get_settings,
            set_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running Workboard");
}
