use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, State, ipc::Channel};
use tokio_util::sync::CancellationToken;

mod auth;
mod provider;
mod secrets;
mod settings;

use auth::AuthStatus;
use provider::{GenerateRequest, ResponseEvent};
use settings::Settings;

struct RuntimeState {
    active_generation: Mutex<Option<CancellationToken>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSnapshot {
    settings: Settings,
    chatgpt: AuthStatus,
    has_api_key: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSettingsRequest {
    settings: Settings,
    api_key: Option<String>,
    change_api_key: bool,
}

#[tauri::command]
fn get_app_snapshot(app: AppHandle) -> Result<AppSnapshot, String> {
    Ok(AppSnapshot {
        settings: settings::load(&app)?,
        chatgpt: auth::status()?,
        has_api_key: secrets::has_api_key()?,
    })
}

#[tauri::command]
fn save_settings(app: AppHandle, request: SaveSettingsRequest) -> Result<AppSnapshot, String> {
    settings::save(&app, &request.settings)?;
    if request.change_api_key {
        secrets::save_api_key(request.api_key.as_deref())?;
    }
    get_app_snapshot(app)
}

#[tauri::command]
async fn list_models(app: AppHandle) -> Result<Vec<provider::ModelOption>, String> {
    provider::list_models(settings::load(&app)?).await
}

#[tauri::command]
async fn start_chatgpt_login(app: AppHandle) -> Result<String, String> {
    auth::start_login(app).await
}

#[tauri::command]
fn logout_chatgpt(app: AppHandle) -> Result<AppSnapshot, String> {
    secrets::delete_oauth()?;
    get_app_snapshot(app)
}

#[tauri::command]
async fn generate_example(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    request: GenerateRequest,
    on_event: Channel<ResponseEvent>,
) -> Result<(), String> {
    let settings = settings::load(&app)?;
    let cancellation = CancellationToken::new();
    {
        let mut active = state
            .active_generation
            .lock()
            .map_err(|_| "Generation state is unavailable".to_owned())?;
        if active.is_some() {
            return Err("Another example is already being generated".to_owned());
        }
        *active = Some(cancellation.clone());
    }

    let result = provider::generate(settings, request, on_event.clone(), cancellation).await;
    if let Ok(mut active) = state.active_generation.lock() {
        *active = None;
    }
    if let Err(message) = result {
        let _ = on_event.send(ResponseEvent::Failed { message });
    }
    Ok(())
}

#[tauri::command]
fn cancel_generation(state: State<'_, RuntimeState>) -> Result<(), String> {
    let active = state
        .active_generation
        .lock()
        .map_err(|_| "Generation state is unavailable".to_owned())?;
    if let Some(cancellation) = active.as_ref() {
        cancellation.cancel();
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(RuntimeState {
            active_generation: Mutex::new(None),
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_app_snapshot,
            save_settings,
            list_models,
            start_chatgpt_login,
            logout_chatgpt,
            generate_example,
            cancel_generation,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Unfold");
}
