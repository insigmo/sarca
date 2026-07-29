use std::{
    fs,
    path::{Path, PathBuf},
};

use sarca_sync::{
    normalize_server_url, Binding, BindingMode, SarcaApi, StorageSummary, SyncStatus,
    TransferQueueSnapshot,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri_plugin_dialog::DialogExt;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tokio::sync::oneshot;

use crate::client_log;
use crate::startup::{
    is_usable_device_label, is_useless_hostname, read_device_label_cache, sanitize_device_label,
    write_device_label_cache,
};
use crate::state::{
    navigate_to_server, navigate_to_shell, navigate_to_sync_settings, new_binding,
    read_webview_session, session_ready_for_sync, AppSyncState, ClientPrefs, ServerConfig,
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
    pub limit_bytes: u64,
}

fn cache_root(state: &AppSyncState) -> PathBuf {
    state.data_dir().join("cache")
}

fn preview_cache_path(state: &AppSyncState, scope: &str, logical_path: &str) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    scope.hash(&mut h);
    logical_path.hash(&mut h);
    "v1-1920-q80".hash(&mut h);
    let digest = format!("{:016x}", h.finish());
    cache_root(state)
        .join("preview")
        .join(sanitize_cache_scope(scope))
        .join(format!("{digest}.jpg"))
}

fn sanitize_cache_scope(scope: &str) -> String {
    scope
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
struct CacheFileEntry {
    path: PathBuf,
    size: u64,
    modified: std::time::SystemTime,
}

fn list_cache_files(root: &Path) -> Vec<CacheFileEntry> {
    let mut entries = Vec::new();
    if !root.exists() {
        return entries;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = entry.metadata() {
                entries.push(CacheFileEntry {
                    path,
                    size: meta.len(),
                    modified: meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                });
            }
        }
    }
    entries
}

fn evict_cache_if_needed(state: &AppSyncState) -> Result<(), String> {
    let prefs = load_prefs(state);
    let limit = prefs.cache_limit_bytes;
    let root = cache_root(state);
    let mut total = dir_size(&root);
    if total <= limit {
        return Ok(());
    }

    let mut entries = list_cache_files(&root);
    entries.sort_by_key(|e| e.modified);
    let mut removed = 0u64;
    for entry in entries {
        if total <= limit {
            break;
        }
        if fs::remove_file(&entry.path).is_ok() {
            total = total.saturating_sub(entry.size);
            removed = removed.saturating_add(entry.size);
        }
    }
    let _ = removed;
    Ok(())
}

fn cache_dto(state: &AppSyncState) -> CacheDto {
    let cache = cache_root(state);
    CacheDto {
        bytes: if cache.exists() { dir_size(&cache) } else { 0 },
        limit_bytes: load_prefs(state).cache_limit_bytes,
    }
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
    fs::write(prefs_path(state), json).map_err(|e| e.to_string())?;
    client_log::set_enabled(prefs.enable_logs, state.data_dir());
    Ok(())
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
///
/// `wifi_only` is a mobile (cellular) concern. On desktop there is no cellular
/// radio, so the preference is a no-op — otherwise a down Wi‑Fi NIC with working
/// ethernet silently skipped Camera uploads forever.
pub fn allow_auto_upload(prefs: &ClientPrefs) -> bool {
    if !prefs.wifi_only {
        return true;
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        is_wifi_connected()
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let _ = prefs;
        true
    }
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
pub fn device_label(app: AppHandle) -> String {
    resolve_device_label(&app)
}

/// Resolve the device label, preferring the on-disk cache; refresh/write cache when needed.
pub fn resolve_device_label(app: &AppHandle) -> String {
    let fallback = platform_label();
    let data_dir = app
        .try_state::<AppSyncState>()
        .map(|s| s.data_dir().clone());

    if let Some(ref dir) = data_dir {
        if let Some(cached) = read_device_label_cache(dir) {
            return cached;
        }
    }

    let live = resolve_live_device_label(app, &fallback);
    if let Some(ref dir) = data_dir {
        if is_usable_device_label(&live) {
            let _ = write_device_label_cache(dir, &live);
        }
    }
    live
}

fn resolve_live_device_label(app: &AppHandle, fallback: &str) -> String {
    #[cfg(target_os = "android")]
    {
        if let Some(label) = crate::startup::device_model_label(app) {
            return label;
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
    }
    let raw = hostname::get()
        .ok()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cleaned = sanitize_device_label(&raw);
    if cleaned.is_empty() || is_useless_hostname(&cleaned) {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// Best-effort: resolve + persist device identity during app startup.
pub fn ensure_device_label_cached(app: &AppHandle) {
    let _ = resolve_device_label(app);
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

/// Push the webview's live access/refresh tokens into native Sync state.
/// Called automatically by `__sarcaInvoke` before Sync API commands.
#[tauri::command]
pub async fn update_session(
    state: State<'_, AppSyncState>,
    access_token: String,
    refresh_token: Option<String>,
    email: Option<String>,
    email_verified: Option<bool>,
) -> Result<SessionDto, String> {
    state
        .apply_webview_session(access_token, refresh_token, email, email_verified)
        .await?;
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
) -> Result<SessionDto, String> {
    let base = normalize_server_url(&server_url).map_err(|e| e.to_string())?;

    // Remember the server origin; authentication happens on the website login page.
    // Keep any existing tokens only when reconnecting to the same base URL.
    let previous = state.server.lock().await.clone();
    let same_server = previous.base_url.trim_end_matches('/') == base.trim_end_matches('/');
    let cfg = ServerConfig {
        base_url: base,
        access_token: if same_server {
            previous.access_token
        } else {
            String::new()
        },
        refresh_token: if same_server {
            previous.refresh_token
        } else {
            String::new()
        },
        email: if same_server {
            previous.email
        } else {
            String::new()
        },
        email_verified: if same_server {
            previous.email_verified
        } else {
            false
        },
    };
    state.save_server(&cfg).await.map_err(|e| e.to_string())?;
    navigate_to_server(&app, &cfg)?;

    Ok(SessionDto {
        connected: cfg.is_connected(),
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

/// Pull live tokens from the webview into native state, then return the server config.
/// Shows "sign in again" only when both native and webview lack an access token.
async fn ensure_sync_session(
    app: &AppHandle,
    state: &AppSyncState,
) -> Result<ServerConfig, String> {
    let _ = state.sync_session_from_webview(app).await;
    let cfg = state.server.lock().await.clone();
    if cfg.is_connected() {
        return Ok(cfg);
    }
    let webview_tokens = read_webview_session(app).await;
    let webview_has = webview_tokens
        .as_ref()
        .map(|t| t.has_access())
        .unwrap_or(false);
    if !session_ready_for_sync(false, webview_has) {
        return Err(
            "Not connected — sign in again so Sync can use your session".into(),
        );
    }
    // Webview had tokens but first apply failed — retry once with explicit apply.
    if let Some(tokens) = webview_tokens {
        state
            .apply_webview_session(
                tokens.access_token,
                tokens.refresh_token,
                tokens.email,
                tokens.email_verified,
            )
            .await?;
        return Ok(state.server.lock().await.clone());
    }
    Err("Not connected — sign in again so Sync can use your session".into())
}

const SESSION_EXPIRED_MSG: &str =
    "Session expired — sign in again so Sync can create remote folders";

fn is_unauthorized(msg: &str) -> bool {
    msg.contains("401") || msg.to_ascii_lowercase().contains("unauthorized")
}

/// On 401: re-pull webview tokens silently, retry; then refresh+retry.
/// Only surface SESSION_EXPIRED_MSG when the webview also has no usable tokens.
async fn create_folder_with_auth_retry(
    app: &AppHandle,
    state: &AppSyncState,
    mut cfg: ServerConfig,
    sid: uuid::Uuid,
    parent: &str,
    name: &str,
) -> Result<(), String> {
    match try_create_folder(&cfg, sid, parent, name).await {
        Ok(()) => return Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if !is_unauthorized(&msg) {
                return Err(msg);
            }
        }
    }

    // Silent re-sync from webview (covers missed watch / JSON-quoted tokens fixed in state).
    if let Some(tokens) = state.sync_session_from_webview(app).await {
        cfg = state.server.lock().await.clone();
        if try_create_folder(&cfg, sid, parent, name).await.is_ok() {
            let _ = tokens;
            return Ok(());
        }
    } else {
        cfg = state.server.lock().await.clone();
    }

    if !cfg.refresh_token.trim().is_empty() {
        match SarcaApi::refresh(&cfg.base_url, &cfg.refresh_token).await {
            Ok(tokens) => {
                cfg.access_token = tokens.access_token;
                cfg.refresh_token = tokens.refresh_token;
                cfg.email_verified = tokens.email_verified;
                state
                    .save_server(&cfg)
                    .await
                    .map_err(|e| e.to_string())?;
                return try_create_folder(&cfg, sid, parent, name)
                    .await
                    .map_err(|e| e.to_string());
            }
            Err(_) => {
                // Fall through — only expire if webview is also empty.
            }
        }
    }

    let webview_has = read_webview_session(app)
        .await
        .map(|t| t.has_access())
        .unwrap_or(false);
    if webview_has {
        // Tokens exist in UI but API still rejects — likely server/permission issue,
        // not a missing client session. Avoid the misleading "Session expired" copy.
        return Err(
            "Could not create remote folder with the current session — try again or re-open Sync"
                .into(),
        );
    }
    Err(SESSION_EXPIRED_MSG.into())
}

#[tauri::command]
pub async fn list_storages(
    app: AppHandle,
    state: State<'_, AppSyncState>,
) -> Result<Vec<StorageDto>, String> {
    let cfg = ensure_sync_session(&app, &state).await?;
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
    app: AppHandle,
    state: State<'_, AppSyncState>,
    storage_id: String,
    parent: String,
    name: String,
) -> Result<String, String> {
    let cfg = ensure_sync_session(&app, &state).await?;
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err("Folder name is required".into());
    }
    let sid = uuid::Uuid::parse_str(&storage_id).map_err(|e| e.to_string())?;
    let parent = parent.trim().trim_matches('/').to_owned();

    create_folder_with_auth_retry(&app, &state, cfg, sid, &parent, &name).await?;

    let remote = if parent.is_empty() {
        name
    } else {
        format!("{parent}/{name}")
    };
    Ok(remote)
}

async fn try_create_folder(
    cfg: &ServerConfig,
    storage_id: uuid::Uuid,
    parent: &str,
    name: &str,
) -> Result<(), anyhow::Error> {
    let api = SarcaApi::new(&cfg.base_url, &cfg.access_token);
    api.create_folder(storage_id, parent, name).await
}

#[tauri::command]
pub fn list_bindings(state: State<'_, AppSyncState>) -> Result<Vec<Binding>, String> {
    state.engine.list_bindings().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_binding(
    app: AppHandle,
    state: State<'_, AppSyncState>,
    storage_id: String,
    remote_root: String,
    local_path: String,
    mode: String,
) -> Result<Binding, String> {
    let _ = ensure_sync_session(&app, &state).await;
    let binding =
        new_binding(&storage_id, remote_root, local_path, &mode).map_err(|e| e.to_string())?;
    // Only one Camera (media) auto-upload binding at a time — UI races used to leave
    // duplicates that re-uploaded the same gallery three times per tick.
    // Folder uploads may be many; they are not deduped here.
    if matches!(binding.mode, BindingMode::AutoUpload) {
        let existing = state.engine.list_bindings().map_err(|e| e.to_string())?;
        for b in existing
            .into_iter()
            .filter(|b| matches!(b.mode, BindingMode::AutoUpload) && b.id != binding.id)
        {
            state
                .engine
                .remove_binding(&b.id)
                .map_err(|e| e.to_string())?;
        }
    }
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
pub fn set_binding_enabled(
    state: State<'_, AppSyncState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    client_log::write_line(
        state.data_dir(),
        &format!("set_binding_enabled id={id} enabled={enabled}"),
    );
    state
        .engine
        .set_binding_enabled(&id, enabled)
        .map_err(|e| e.to_string())
}

/// Two-way `Sync` bindings track per-file state (content hash, remote file
/// id, cursor) keyed to the *old* local root; repointing them at a different
/// folder would desync every entry (spurious deletes/uploads on the next
/// tick). Upload-only bindings (Camera / folder auto-upload) are safe to
/// repoint: `push_local` just re-walks the new root and its existing
/// size/mtime/hash comparisons in the index still protect against
/// accidental re-uploads of files that happen to already match.
fn ensure_local_path_change_allowed(mode: BindingMode) -> Result<(), String> {
    if mode.is_upload_only() {
        Ok(())
    } else {
        Err("Changing the local folder is only supported for upload-only bindings".into())
    }
}

fn ensure_remote_root_change_allowed(mode: BindingMode) -> Result<(), String> {
    if mode.is_upload_only() {
        Ok(())
    } else {
        Err("Changing the remote folder is only supported for upload-only bindings".into())
    }
}

#[tauri::command]
pub fn update_binding_local_path(
    state: State<'_, AppSyncState>,
    id: String,
    local_path: String,
) -> Result<Binding, String> {
    let mut binding = state
        .engine
        .list_bindings()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.id == id)
        .ok_or_else(|| format!("binding not found: {id}"))?;
    ensure_local_path_change_allowed(binding.mode)?;
    binding.local_path = local_path;
    state
        .engine
        .upsert_binding(&binding)
        .map_err(|e| e.to_string())?;
    Ok(binding)
}

#[tauri::command]
pub fn update_binding_remote_root(
    state: State<'_, AppSyncState>,
    id: String,
    remote_root: String,
) -> Result<Binding, String> {
    let mut binding = state
        .engine
        .list_bindings()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.id == id)
        .ok_or_else(|| format!("binding not found: {id}"))?;
    ensure_remote_root_change_allowed(binding.mode)?;
    binding.remote_root = remote_root.trim().trim_matches('/').to_owned();
    state
        .engine
        .upsert_binding(&binding)
        .map_err(|e| e.to_string())?;
    Ok(binding)
}

#[tauri::command]
pub async fn sync_now(
    app: AppHandle,
    state: State<'_, AppSyncState>,
    binding_id: Option<String>,
) -> Result<(), String> {
    let _ = ensure_sync_session(&app, &state).await;
    let prefs = load_prefs(&state);
    let allow_auto = allow_auto_upload(&prefs);
    let allow = |b: &Binding| {
        if b.mode.is_upload_only() && !allow_auto {
            return false;
        }
        true
    };
    match binding_id {
        Some(id) => state
            .engine
            .tick_binding(&id, allow)
            .await
            .map_err(|e| e.to_string()),
        None => state
            .engine
            .tick_filtered(allow)
            .await
            .map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn sync_statuses(state: State<'_, AppSyncState>) -> Result<Vec<SyncStatus>, String> {
    Ok(state.engine.statuses().await)
}

#[tauri::command]
pub async fn sync_transfer_queue(
    state: State<'_, AppSyncState>,
) -> Result<TransferQueueSnapshot, String> {
    Ok(state.engine.transfer_queue().await)
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

#[derive(Serialize)]
pub struct ExportLogsDto {
    pub path: String,
    pub shared: bool,
    /// Log text for desktop download / clipboard when share sheet is unavailable.
    pub content: String,
}

/// Export client logs: Android opens a share sheet; desktop returns path + content.
#[tauri::command]
pub async fn export_logs(
    app: AppHandle,
    state: State<'_, AppSyncState>,
) -> Result<ExportLogsDto, String> {
    let data_dir = state.data_dir().clone();
    // Ensure the file exists so share/save always has something.
    if !client_log::is_enabled() {
        client_log::set_enabled(true, &data_dir);
        client_log::write_line(&data_dir, "export_logs: logging was off; enabled for export");
        let mut prefs = load_prefs(&state);
        prefs.enable_logs = true;
        let _ = save_prefs(&state, &prefs);
    }
    let text = client_log::read_export(&data_dir, 512 * 1024)?;
    #[cfg(target_os = "android")]
    {
        let path = client_log::log_path(&data_dir);
        crate::startup::share_text(&app, &text, "Sarca client logs").await?;
        return Ok(ExportLogsDto {
            path: path.display().to_string(),
            shared: true,
            content: text,
        });
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        let export = data_dir.join("logs").join("sarca-client-export.log");
        fs::write(&export, &text).map_err(|e| e.to_string())?;
        Ok(ExportLogsDto {
            path: export.display().to_string(),
            shared: false,
            content: text,
        })
    }
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
    Ok(cache_dto(&state))
}

#[tauri::command]
pub fn clear_local_cache(state: State<'_, AppSyncState>) -> Result<CacheDto, String> {
    let cache = cache_root(&state);
    fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    remove_dir_contents(&cache)?;
    Ok(CacheDto {
        bytes: 0,
        limit_bytes: load_prefs(&state).cache_limit_bytes,
    })
}

#[tauri::command]
pub fn cache_get_preview(
    state: State<'_, AppSyncState>,
    scope: String,
    path: String,
) -> Result<Option<String>, String> {
    let dest = preview_cache_path(&state, &scope, &path);
    if !dest.is_file() {
        return Ok(None);
    }
    if let Ok(f) = fs::OpenOptions::new().write(true).open(&dest) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
    let bytes = fs::read(&dest).map_err(|e| e.to_string())?;
    Ok(Some(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        bytes,
    )))
}

#[tauri::command]
pub fn cache_put_preview(
    state: State<'_, AppSyncState>,
    scope: String,
    path: String,
    bytes_b64: String,
) -> Result<(), String> {
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, bytes_b64)
        .map_err(|e| format!("invalid preview cache payload: {e}"))?;
    let dest = preview_cache_path(&state, &scope, &path);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = dest.with_extension("tmp");
    fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    if dest.is_file() {
        let _ = fs::remove_file(&dest);
    }
    fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    evict_cache_if_needed(&state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn client_prefs_default_cache_limit_one_gib() {
        let prefs = ClientPrefs::default();
        assert_eq!(prefs.cache_limit_bytes, 1_073_741_824);
    }

    #[test]
    fn client_prefs_missing_cache_limit_deserializes_default() {
        let prefs: ClientPrefs = serde_json::from_str(r#"{"wifi_only":true}"#).unwrap();
        assert_eq!(prefs.cache_limit_bytes, 1_073_741_824);
    }

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

    #[test]
    fn ensure_remote_folder_path_uses_connected_session_tokens() {
        // Regression: create Camera/ must use ServerConfig.access_token (the same
        // session update_session writes from the logged-in webview).
        let cfg = ServerConfig {
            base_url: "http://localhost:8001".into(),
            access_token: "webview-live-token".into(),
            refresh_token: "webview-refresh".into(),
            email: "user@example.com".into(),
            email_verified: true,
        };
        assert!(cfg.is_connected());
        let api = SarcaApi::new(&cfg.base_url, &cfg.access_token);
        assert_eq!(
            api.authorization_header().as_deref(),
            Some("Bearer webview-live-token")
        );
    }

    #[test]
    fn ensure_remote_folder_rejects_disconnected_session() {
        let cfg = ServerConfig {
            base_url: "http://localhost:8001".into(),
            access_token: String::new(),
            ..Default::default()
        };
        assert!(!cfg.is_connected());
        let api = SarcaApi::new(&cfg.base_url, &cfg.access_token);
        assert_eq!(api.authorization_header(), None);
    }

    #[test]
    fn ensure_path_refuses_only_when_native_and_webview_empty() {
        use crate::state::{
            merge_session_tokens, session_ready_for_sync, WebviewSessionTokens,
        };

        // Both empty → refuse
        assert!(!session_ready_for_sync(false, false));

        // Webview has JSON-encoded tokens → treat as ready (pull will update native)
        let webview = WebviewSessionTokens::from_local_storage_raw(
            Some("\"pulled-access\""),
            Some("\"pulled-refresh\""),
            None,
        )
        .expect("webview tokens");
        assert!(session_ready_for_sync(false, webview.has_access()));

        // Pulling into native updates state (simulates ensure_sync_session apply)
        let mut cfg = ServerConfig {
            base_url: "http://localhost:8001".into(),
            ..Default::default()
        };
        assert!(!cfg.is_connected());
        merge_session_tokens(
            &mut cfg,
            &webview.access_token,
            webview.refresh_token.as_deref(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(cfg.access_token, "pulled-access");
        assert_eq!(cfg.refresh_token, "pulled-refresh");
        assert!(cfg.is_connected());
        let api = SarcaApi::new(&cfg.base_url, &cfg.access_token);
        assert_eq!(
            api.authorization_header().as_deref(),
            Some("Bearer pulled-access"),
            "must not send JSON quotes in Authorization header"
        );
    }

    #[test]
    fn session_expired_message_is_stable() {
        assert!(SESSION_EXPIRED_MSG.contains("Session expired"));
        assert!(is_unauthorized("create_folder failed: 401 Unauthorized"));
        assert!(!is_unauthorized("create_folder failed: 500"));
    }

    #[test]
    fn update_binding_local_path_rejects_two_way_sync_bindings() {
        assert!(ensure_local_path_change_allowed(BindingMode::AutoUpload).is_ok());
        assert!(ensure_local_path_change_allowed(BindingMode::FolderUpload).is_ok());
        let err = ensure_local_path_change_allowed(BindingMode::Sync).unwrap_err();
        assert!(
            err.contains("upload-only"),
            "error should explain why Sync bindings are rejected: {err}"
        );
    }

    #[test]
    fn desktop_wifi_only_does_not_block_auto_upload() {
        let prefs = ClientPrefs {
            wifi_only: true,
            ..Default::default()
        };
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            assert!(
                allow_auto_upload(&prefs),
                "desktop must not silently skip auto-upload when wifi_only=true"
            );
        }
    }
}
