use sarca_sync::{Binding, SyncStatus};
use serde::Serialize;
use tauri::State;

use crate::state::{new_binding, AppSyncState, ServerConfig};

#[derive(Serialize)]
pub struct ServerConfigDto {
    pub base_url: String,
    pub access_token: String,
}

#[tauri::command]
pub fn platform_label() -> String {
    #[cfg(target_os = "android")]
    {
        return "Android".into();
    }
    #[cfg(target_os = "ios")]
    {
        return "iOS".into();
    }
    #[cfg(target_os = "macos")]
    {
        return "macOS".into();
    }
    #[cfg(target_os = "windows")]
    {
        return "Windows".into();
    }
    #[cfg(target_os = "linux")]
    {
        return "Linux".into();
    }
    #[cfg(not(any(
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    )))]
    {
        "Unknown".into()
    }
}

#[tauri::command]
pub async fn get_server_config(state: State<'_, AppSyncState>) -> Result<ServerConfigDto, String> {
    let cfg = state.server.lock().await.clone();
    Ok(ServerConfigDto {
        base_url: cfg.base_url,
        access_token: cfg.access_token,
    })
}

#[tauri::command]
pub async fn set_server_config(
    state: State<'_, AppSyncState>,
    base_url: String,
    access_token: String,
) -> Result<(), String> {
    let cfg = ServerConfig {
        base_url,
        access_token,
    };
    state.save_server(&cfg).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_bindings(state: State<'_, AppSyncState>) -> Result<Vec<Binding>, String> {
    state.engine.list_bindings().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_binding(
    state: State<'_, AppSyncState>,
    storage_id: String,
    remote_root: String,
    local_path: String,
    mode: String,
) -> Result<Binding, String> {
    let binding =
        new_binding(&storage_id, remote_root, local_path, &mode).map_err(|e| e.to_string())?;
    state
        .engine
        .upsert_binding(&binding)
        .map_err(|e| e.to_string())?;
    Ok(binding)
}

#[tauri::command]
pub fn remove_binding(state: State<'_, AppSyncState>, id: String) -> Result<(), String> {
    state.engine.remove_binding(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sync_now(state: State<'_, AppSyncState>) -> Result<(), String> {
    state.engine.tick().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sync_statuses(state: State<'_, AppSyncState>) -> Result<Vec<SyncStatus>, String> {
    Ok(state.engine.statuses().await)
}
