use std::{
    fs,
    path::{Path, PathBuf},
};

use sarca_sync::{normalize_server_url, Binding, BindingMode, SarcaApi, StorageSummary, SyncStatus};
use serde::Serialize;
use tauri::{AppHandle, State};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri_plugin_dialog::DialogExt;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tokio::sync::oneshot;

use crate::state::{
    navigate_to_server, navigate_to_shell, navigate_to_sync_settings, new_binding, AppSyncState,
    ClientPrefs, ServerConfig,
};

#[derive(Serialize)]
pub struct SessionDto {
    pub connected: bool,
    pub base_url: String,
    pub email: String,
}

#[derive(Serialize)]
pub struct StorageDto {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct AboutDto {
    pub version: String,
    pub platform: String,
}

#[derive(Serialize)]
pub struct CacheDto {
    pub bytes: u64,
}

fn prefs_path(state: &AppSyncState) -> PathBuf {
    state.data_dir().join("client_prefs.json")
}

pub fn load_prefs(state: &AppSyncState) -> ClientPrefs {
    fs::read_to_string(prefs_path(state))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_prefs(state: &AppSyncState, prefs: &ClientPrefs) -> Result<(), String> {
    let json = serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?;
    fs::write(prefs_path(state), json).map_err(|e| e.to_string())
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            total = total.saturating_add(dir_size(&p));
        } else if let Ok(meta) = entry.metadata() {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

fn remove_dir_contents(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if p.is_dir() {
            fs::remove_dir_all(&p).map_err(|e| e.to_string())?;
        } else {
            fs::remove_file(&p).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn is_wifi_connected() -> bool {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        use network_interface::{NetworkInterface, NetworkInterfaceConfig};
        let Ok(list) = NetworkInterface::show() else {
            return true;
        };
        let has_wifi_name = list.iter().any(|iface| {
            let n = iface.name.to_lowercase();
            n.starts_with("wl")
                || n.contains("wlan")
                || n.contains("wifi")
                || n.contains("wi-fi")
                || n.contains("airport")
        });
        if !has_wifi_name {
            // No Wi‑Fi interface on this host (desktop ethernet-only, etc.) — allow uploads.
            return true;
        }
        list.iter().any(|iface| {
            let n = iface.name.to_lowercase();
            let looks_wifi = n.starts_with("wl")
                || n.contains("wlan")
                || n.contains("wifi")
                || n.contains("wi-fi")
                || n.contains("airport");
            looks_wifi && !iface.addr.is_empty()
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        true
    }
}

/// Whether auto-upload bindings should run given prefs + connectivity.
pub fn allow_auto_upload(prefs: &ClientPrefs) -> bool {
    if !prefs.wifi_only {
        return true;
    }
    is_wifi_connected()
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
    let base = normalize_server_url(&server_url).map_err(|e| e.to_string())?;
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
pub fn open_sync_settings(app: AppHandle) -> Result<(), String> {
    navigate_to_sync_settings(&app)
}

/// Map a successful desktop folder-dialog path to the string the UI expects.
/// Kept separate so tests can prove we never return the prompt sentinel on success.
pub fn folder_path_from_picked_path(path: Option<std::path::PathBuf>) -> Option<String> {
    path.map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn pick_local_folder(app: AppHandle) -> Result<Option<String>, String> {
    // Desktop: native OS folder dialog (async, non-blocking).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let (tx, rx) = oneshot::channel();
        app.dialog()
            .file()
            .set_title("Choose folder")
            .pick_folder(move |folder| {
                let _ = tx.send(folder);
            });
        let folder = match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(folder)) => folder,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => return Err("Folder picker timed out".into()),
        };
        return Ok(folder_path_from_picked_path(
            folder.and_then(|p| p.into_path().ok()),
        ));
    }

    // Android: SAF document-tree picker → filesystem path when resolvable.
    #[cfg(target_os = "android")]
    {
        return crate::folder_picker::pick_folder_android(&app).await;
    }

    // iOS: no reliable folder path for walkdir yet — typed path fallback.
    #[cfg(target_os = "ios")]
    {
        let _ = app;
        Err("FOLDER_PICKER_USE_PROMPT".into())
    }
}

#[tauri::command]
pub fn default_gallery_path() -> String {
    #[cfg(target_os = "android")]
    {
        return "/storage/emulated/0/DCIM".into();
    }
    #[cfg(target_os = "ios")]
    {
        return "".into();
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        std::env::var("HOME")
            .map(|h| format!("{h}/Pictures"))
            .unwrap_or_else(|_| "Pictures".into())
    }
}

#[tauri::command]
pub async fn list_storages(state: State<'_, AppSyncState>) -> Result<Vec<StorageDto>, String> {
    let cfg = state.server.lock().await.clone();
    if !cfg.is_connected() {
        return Err("Not connected".into());
    }
    let api = SarcaApi::new(&cfg.base_url, &cfg.access_token);
    let list = api.list_storages().await.map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|s: StorageSummary| StorageDto {
            id: s.id.to_string(),
            name: s.name,
        })
        .collect())
}

#[tauri::command]
pub async fn ensure_remote_folder(
    state: State<'_, AppSyncState>,
    storage_id: String,
    parent: String,
    name: String,
) -> Result<String, String> {
    let cfg = state.server.lock().await.clone();
    if !cfg.is_connected() {
        return Err("Not connected".into());
    }
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err("Folder name is required".into());
    }
    let sid = uuid::Uuid::parse_str(&storage_id).map_err(|e| e.to_string())?;
    let parent = parent.trim().trim_matches('/').to_owned();
    let api = SarcaApi::new(&cfg.base_url, &cfg.access_token);
    api.create_folder(sid, &parent, &name)
        .await
        .map_err(|e| e.to_string())?;
    let remote = if parent.is_empty() {
        name
    } else {
        format!("{parent}/{name}")
    };
    Ok(remote)
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
    let prefs = load_prefs(&state);
    let allow_auto = allow_auto_upload(&prefs);
    state
        .engine
        .tick_filtered(|b| {
            if matches!(b.mode, BindingMode::AutoUpload) && !allow_auto {
                return false;
            }
            true
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sync_statuses(state: State<'_, AppSyncState>) -> Result<Vec<SyncStatus>, String> {
    Ok(state.engine.statuses().await)
}

#[tauri::command]
pub fn get_client_prefs(state: State<'_, AppSyncState>) -> Result<ClientPrefs, String> {
    Ok(load_prefs(&state))
}

#[tauri::command]
pub fn set_client_prefs(
    state: State<'_, AppSyncState>,
    prefs: ClientPrefs,
) -> Result<ClientPrefs, String> {
    save_prefs(&state, &prefs)?;
    Ok(prefs)
}

#[tauri::command]
pub fn is_on_wifi() -> bool {
    is_wifi_connected()
}

#[tauri::command]
pub fn get_about() -> AboutDto {
    AboutDto {
        version: env!("CARGO_PKG_VERSION").into(),
        platform: platform_label(),
    }
}

#[tauri::command]
pub fn get_cache_size(state: State<'_, AppSyncState>) -> Result<CacheDto, String> {
    let cache = state.data_dir().join("cache");
    Ok(CacheDto {
        bytes: if cache.exists() { dir_size(&cache) } else { 0 },
    })
}

#[tauri::command]
pub fn clear_local_cache(state: State<'_, AppSyncState>) -> Result<CacheDto, String> {
    let cache = state.data_dir().join("cache");
    fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    remove_dir_contents(&cache)?;
    Ok(CacheDto { bytes: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn desktop_dialog_success_returns_path_not_prompt_sentinel() {
        let path = folder_path_from_picked_path(Some(PathBuf::from("/home/beta/Pictures")));
        assert_eq!(path.as_deref(), Some("/home/beta/Pictures"));
        assert!(
            !path.unwrap().contains("FOLDER_PICKER_USE_PROMPT"),
            "successful dialog must never return the typed-path sentinel"
        );
        assert_eq!(folder_path_from_picked_path(None), None);
    }

    #[test]
    fn default_gallery_path_non_empty_on_desktop() {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let p = default_gallery_path();
            assert!(!p.is_empty());
            assert!(
                p.contains("Pictures") || p == "Pictures",
                "unexpected gallery path {p}"
            );
        }
    }
}
