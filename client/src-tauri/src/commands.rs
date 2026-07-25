use sarca_sync::{Binding, SarcaApi, SyncStatus};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::state::{
    navigate_to_server, navigate_to_shell, new_binding, AppSyncState, ServerConfig,
};

#[derive(Serialize)]
pub struct SessionDto {
    pub connected: bool,
    pub base_url: String,
    pub email: String,
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
pub async fn get_session(state: State<'_, AppSyncState>) -> Result<SessionDto, String> {
    let cfg = state.server.lock().await.clone();
    Ok(SessionDto {
        connected: cfg.is_connected(),
        base_url: cfg.base_url,
        email: cfg.email,
    })
}

#[tauri::command]
pub async fn connect(
    app: AppHandle,
    state: State<'_, AppSyncState>,
    server_url: String,
    email: String,
    password: String,
) -> Result<SessionDto, String> {
    let base = server_url.trim().trim_end_matches('/').to_owned();
    if base.is_empty() {
        return Err("Server URL is required".into());
    }
    if email.trim().is_empty() {
        return Err("Email is required".into());
    }
    if password.is_empty() {
        return Err("Password is required".into());
    }

    let tokens = SarcaApi::login(&base, email.trim(), &password)
        .await
        .map_err(|e| e.to_string())?;

    let cfg = ServerConfig {
        base_url: base,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        email: email.trim().to_owned(),
        email_verified: tokens.email_verified,
    };
    state.save_server(&cfg).await.map_err(|e| e.to_string())?;
    navigate_to_server(&app, &cfg)?;

    Ok(SessionDto {
        connected: true,
        base_url: cfg.base_url,
        email: cfg.email,
    })
}

#[tauri::command]
pub async fn disconnect(app: AppHandle, state: State<'_, AppSyncState>) -> Result<(), String> {
    let cfg = ServerConfig::default();
    state.save_server(&cfg).await.map_err(|e| e.to_string())?;
    if let Ok(mut guard) = state.pending_inject.lock() {
        *guard = None;
    }
    navigate_to_shell(&app)
}

#[tauri::command]
pub async fn open_app(app: AppHandle, state: State<'_, AppSyncState>) -> Result<(), String> {
    let cfg = state.server.lock().await.clone();
    if !cfg.is_connected() {
        return Err("Not connected".into());
    }
    navigate_to_server(&app, &cfg)
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
