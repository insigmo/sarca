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
use crate::paths::validate_local_dir;
use crate::startup::{
    is_usable_device_label, is_useless_hostname, read_device_label_cache, sanitize_device_label,
    write_device_label_cache,
};
use crate::state::{
    navigate_to_server, navigate_to_shell, navigate_to_sync_settings, new_binding,
    read_webview_session, session_ready_for_sync, write_private, AppSyncState, ClientPrefs,
    ClientPrefsDto, ServerConfig,
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
    // SHA-256 over length-prefixed fields, not `DefaultHasher`. The scope and the
    // path both arrive from the WebView, and std's hasher is a 64-bit SipHash
    // with fixed, public keys: a caller could search out a second (scope, path)
    // that lands on another entry's file and serve a poisoned preview in its
    // place. Length prefixes stop `("a", "bc")` and `("ab", "c")` from hashing
    // to the same slot.
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for field in [scope, logical_path, "v1-1920-q80"] {
        h.update((field.len() as u64).to_le_bytes());
        h.update(field.as_bytes());
    }
    let digest = hex::encode(&h.finalize()[..16]);
    cache_root(state)
        .join("preview")
        .join(sanitize_cache_scope(scope))
        .join(format!("{digest}.jpg"))
}

/// Ceiling for a cached preview: 8 MiB decoded, plus base64's 4/3 expansion.
const MAX_PREVIEW_BYTES: usize = 8 * 1024 * 1024;
const MAX_PREVIEW_B64_LEN: usize = MAX_PREVIEW_BYTES / 3 * 4 + 4;
/// Cache keys are hashed, so their length only bounds the work we do.
const MAX_CACHE_KEY_LEN: usize = 4096;

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
    let mut prefs: ClientPrefs = fs::read_to_string(prefs_path(state))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // Upgrade a file written before the PIN was hashed, so the plaintext stops
    // existing on disk the first time the app reads it.
    if prefs.migrate_legacy_pin() {
        let _ = save_prefs(state, &prefs);
    }
    prefs
}

fn save_prefs(state: &AppSyncState, prefs: &ClientPrefs) -> Result<(), String> {
    let json = serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?;
    // Holds the app-lock hash: keep it out of reach of other local accounts.
    write_private(&prefs_path(state), json.as_bytes()).map_err(|e| e.to_string())?;
    client_log::set_enabled(prefs.enable_logs, state.data_dir());
    Ok(())
}

/// Roots a sync binding's local folder may live under.
///
/// Anything outside them is refused by [`validate_local_dir`], so a hostile
/// `add_binding` cannot point the engine at `/etc`, another user's home, or a
/// mounted network share.
fn allowed_sync_roots(app: &AppHandle) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(home) = app.path().home_dir() {
        roots.push(home);
    }
    #[cfg(target_os = "android")]
    {
        // Android shared storage is not under the app's HOME.
        for root in ["/storage/emulated/0", "/sdcard"] {
            let path = PathBuf::from(root);
            if path.is_dir() {
                roots.push(path);
            }
        }
    }
    roots.retain(|p| p.is_dir());
    roots
}

/// Application-owned directories: binding one would upload the session tokens
/// and the sync database itself.
fn denied_sync_roots(app: &AppHandle, state: &AppSyncState) -> Vec<PathBuf> {
    let path = app.path();
    let mut roots = vec![state.data_dir().clone()];
    for dir in [
        path.app_data_dir(),
        path.app_config_dir(),
        path.app_local_data_dir(),
        path.app_cache_dir(),
    ]
    .into_iter()
    .flatten()
    {
        roots.push(dir);
    }
    roots
}

fn check_local_path(app: &AppHandle, state: &AppSyncState, raw: &str) -> Result<String, String> {
    validate_local_dir(
        raw,
        &allowed_sync_roots(app),
        &denied_sync_roots(app, state),
    )
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
        "Linux".into()
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

/// Recent server URLs for the Connect screen, newest first (max 3).
#[tauri::command]
pub async fn get_url_history(state: State<'_, AppSyncState>) -> Result<Vec<String>, String> {
    Ok(state.load_url_history())
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
    client_log::write_line(
        state.data_dir(),
        &format!("update_session email={email:?} email_verified={email_verified:?}"),
    );
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
    client_log::write_line(
        state.data_dir(),
        &format!("connect server_url={server_url}"),
    );
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
    navigate_to_server(&app, &cfg).await?;
    state.record_url_history(&cfg.base_url);

    Ok(SessionDto {
        connected: cfg.is_connected(),
        base_url: cfg.base_url,
        email: cfg.email,
    })
}

#[tauri::command]
pub async fn disconnect(app: AppHandle, state: State<'_, AppSyncState>) -> Result<(), String> {
    client_log::write_line(state.data_dir(), "disconnect");
    let cfg = ServerConfig::default();
    state.save_server(&cfg).await.map_err(|e| e.to_string())?;
    if let Ok(mut guard) = state.pending_inject.lock() {
        *guard = None;
    }
    navigate_to_shell(&app)
}

#[tauri::command]
pub async fn open_app(app: AppHandle, state: State<'_, AppSyncState>) -> Result<(), String> {
    client_log::write_line(state.data_dir(), "open_app");
    let cfg = state.server.lock().await.clone();
    if !cfg.is_connected() {
        return Err("Not connected".into());
    }
    navigate_to_server(&app, &cfg).await
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
pub async fn pick_local_folder(
    app: AppHandle,
    current: Option<String>,
) -> Result<Option<String>, String> {
    // Desktop: native OS folder dialog (async, non-blocking).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let (tx, rx) = oneshot::channel();
        let mut builder = app.dialog().file().set_title("Choose folder");
        // Start in the folder we already sync. Without this the XDG portal
        // opens on "Recent", which on Linux means enumerating recently-used
        // entries and every gvfs mount before the window appears — seconds of
        // apparent hang on the very dialog the user just asked for.
        if let Some(dir) = current
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .filter(|d| d.is_dir())
        {
            builder = builder.set_directory(dir);
        }
        builder.pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
        let folder = match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(folder)) => folder,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => return Err("Folder picker timed out".into()),
        };
        Ok(folder_path_from_picked_path(
            folder.and_then(|p| p.into_path().ok()),
        ))
    }

    // Android: SAF document-tree picker → filesystem path when resolvable.
    // SAF has no start-directory hint we can honour, so `current` is unused.
    #[cfg(target_os = "android")]
    {
        let _ = current;
        return crate::folder_picker::pick_folder_android(&app).await;
    }

    // iOS: no reliable folder path for walkdir yet — typed path fallback.
    #[cfg(target_os = "ios")]
    {
        let _ = (app, current);
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
        return Err("Not connected — sign in again so Sync can use your session".into());
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
                state.save_server(&cfg).await.map_err(|e| e.to_string())?;
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
    client_log::write_line(state.data_dir(), "list_storages");
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
    client_log::write_line(
        state.data_dir(),
        &format!("ensure_remote_folder storage_id={storage_id} parent={parent} name={name}"),
    );
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

/// Runs a blocking index (SQLite) call off the UI thread.
///
/// Tauri dispatches *synchronous* commands on the main thread, so any command
/// that waits on the index mutex freezes the whole window whenever a sync tick
/// holds it — that is what made reopening Settings, flipping the auto-upload
/// toggle and even opening the GTK folder dialog hang for seconds. Every
/// index-touching command below is `async` + `spawn_blocking` for that reason.
async fn on_engine<T, F>(state: &State<'_, AppSyncState>, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&sarca_sync::SyncEngine) -> Result<T, String> + Send + 'static,
{
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || f(&engine))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_bindings(state: State<'_, AppSyncState>) -> Result<Vec<Binding>, String> {
    on_engine(&state, |engine| {
        engine.list_bindings().map_err(|e| e.to_string())
    })
    .await
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
    client_log::write_line(
        state.data_dir(),
        &format!(
            "add_binding storage_id={storage_id} remote_root={remote_root} \
             local_path={local_path} mode={mode}"
        ),
    );
    // The folder arrives from the WebView. Canonicalize and confine it before
    // the engine starts walking (and uploading) whatever it points at.
    let local_path = check_local_path(&app, &state, &local_path)?;
    let _ = ensure_sync_session(&app, &state).await;
    let binding =
        new_binding(&storage_id, remote_root, local_path, &mode).map_err(|e| e.to_string())?;
    // Only one Camera (media) auto-upload binding at a time — UI races used to leave
    // duplicates that re-uploaded the same gallery three times per tick.
    // Folder uploads may be many; they are not deduped here.
    let dedupe = matches!(binding.mode, BindingMode::AutoUpload);
    let stored = binding.clone();
    on_engine(&state, move |engine| {
        if dedupe {
            let existing = engine.list_bindings().map_err(|e| e.to_string())?;
            for b in existing
                .into_iter()
                .filter(|b| matches!(b.mode, BindingMode::AutoUpload) && b.id != stored.id)
            {
                engine.remove_binding(&b.id).map_err(|e| e.to_string())?;
            }
        }
        engine.upsert_binding(&stored).map_err(|e| e.to_string())
    })
    .await?;
    Ok(binding)
}

#[tauri::command]
pub async fn remove_binding(state: State<'_, AppSyncState>, id: String) -> Result<(), String> {
    client_log::write_line(state.data_dir(), &format!("remove_binding id={id}"));
    on_engine(&state, move |engine| {
        engine.remove_binding(&id).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn set_binding_enabled(
    state: State<'_, AppSyncState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    client_log::write_line(
        state.data_dir(),
        &format!("set_binding_enabled id={id} enabled={enabled}"),
    );
    on_engine(&state, move |engine| {
        engine
            .set_binding_enabled(&id, enabled)
            .map_err(|e| e.to_string())
    })
    .await
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
pub async fn update_binding_local_path(
    app: AppHandle,
    state: State<'_, AppSyncState>,
    id: String,
    local_path: String,
) -> Result<Binding, String> {
    client_log::write_line(
        state.data_dir(),
        &format!("update_binding_local_path id={id} local_path={local_path}"),
    );
    let local_path = check_local_path(&app, &state, &local_path)?;
    on_engine(&state, move |engine| {
        let mut binding = engine
            .list_bindings()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("binding not found: {id}"))?;
        ensure_local_path_change_allowed(binding.mode)?;
        binding.local_path = local_path;
        engine.upsert_binding(&binding).map_err(|e| e.to_string())?;
        Ok(binding)
    })
    .await
}

#[tauri::command]
pub async fn update_binding_remote_root(
    state: State<'_, AppSyncState>,
    id: String,
    remote_root: String,
) -> Result<Binding, String> {
    client_log::write_line(
        state.data_dir(),
        &format!("update_binding_remote_root id={id} remote_root={remote_root}"),
    );
    on_engine(&state, move |engine| {
        let mut binding = engine
            .list_bindings()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("binding not found: {id}"))?;
        ensure_remote_root_change_allowed(binding.mode)?;
        binding.remote_root = remote_root.trim().trim_matches('/').to_owned();
        engine.upsert_binding(&binding).map_err(|e| e.to_string())?;
        Ok(binding)
    })
    .await
}

#[tauri::command]
pub async fn sync_now(
    app: AppHandle,
    state: State<'_, AppSyncState>,
    binding_id: Option<String>,
) -> Result<(), String> {
    client_log::write_line(
        state.data_dir(),
        &format!("sync_now binding_id={binding_id:?}"),
    );
    let _ = ensure_sync_session(&app, &state).await;
    // Re-check (and, if needed, re-prompt for) media permission before every
    // manual sync: the app-startup prompt is fire-and-forget from the
    // caller's perspective, so a user who taps "Sync now" while the system
    // dialog is still up (or was previously dismissed) gets another chance
    // here instead of silently scanning zero files.
    #[cfg(target_os = "android")]
    {
        if let Err(e) = crate::startup::ensure_runtime_access(&app).await {
            tracing::warn!(error = %e, "ensure_runtime_access before sync_now failed");
        }
    }
    let prefs = load_prefs(&state);
    let allow_auto = allow_auto_upload(&prefs);
    let allow = |b: &Binding| {
        if b.mode.is_upload_only() && !allow_auto {
            return false;
        }
        true
    };
    // This is the user asking, explicitly, right now — so drop the retry
    // backoff first. A file deferred for hours because it kept failing is
    // exactly what someone pressing "Upload now" wants reconsidered, and they
    // cannot see the deadline to wait it out. Background ticks never do this.
    let retry_id = binding_id.clone();
    let retried = on_engine(&state, move |engine| {
        engine
            .retry_failed_uploads(retry_id.as_deref())
            .map_err(|e| e.to_string())
    })
    .await;
    match retried {
        Ok(0) => {}
        Ok(n) => client_log::write_line(
            state.data_dir(),
            &format!("sync_now cleared retry backoff for {n} file(s)"),
        ),
        Err(e) => tracing::warn!(error = %e, "clearing upload backoff before sync_now failed"),
    }
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

/// Mirrors the webview's `visibilitychange`/heartbeat state into native sync
/// state, so the background loop can poll fast while someone is watching and
/// back off once they're not (Tauri v2 has no `RunEvent::Paused` to derive
/// this from directly — see `AppSyncState::is_foreground`).
#[tauri::command]
pub fn set_app_foreground(state: State<'_, AppSyncState>, active: bool) -> Result<(), String> {
    state.set_foreground(active);
    Ok(())
}

#[tauri::command]
pub fn get_client_prefs(state: State<'_, AppSyncState>) -> Result<ClientPrefsDto, String> {
    Ok(load_prefs(&state).to_dto())
}

#[tauri::command]
pub fn set_client_prefs(
    state: State<'_, AppSyncState>,
    prefs: ClientPrefsDto,
) -> Result<ClientPrefsDto, String> {
    let mut stored = load_prefs(&state);
    stored.apply_dto(prefs)?;
    save_prefs(&state, &stored)?;
    // The writer reads a process-global flag, set from the stored prefs at
    // startup. Without this the toggle only took effect on the next launch:
    // turning logging on wrote nothing, turning it off kept writing.
    client_log::set_enabled(stored.enable_logs, state.data_dir());
    client_log::write_line(state.data_dir(), "set_client_prefs saved");
    Ok(stored.to_dto())
}

/// Check an app-lock PIN.
///
/// The comparison lives here because the PIN never leaves the Rust side; the
/// WebView used to receive it from `get_client_prefs` and compare in JS, which
/// meant reading the lock screen's own secret was enough to walk past it.
#[tauri::command]
pub fn verify_app_lock_pin(state: State<'_, AppSyncState>, pin: String) -> Result<bool, String> {
    let prefs = load_prefs(&state);
    if !prefs.app_lock_enabled || !prefs.has_pin() {
        return Ok(true);
    }
    // Rate-limit guessing. A 4-digit PIN is 10k combinations; without a delay a
    // script walks the whole space in well under a second.
    std::thread::sleep(std::time::Duration::from_millis(250));
    Ok(prefs.verify_pin(&pin))
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
        client_log::write_line(
            &data_dir,
            "export_logs: logging was off; enabled for export",
        );
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
    client_log::write_line(state.data_dir(), "clear_local_cache");
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
    // Same bound as `cache_put_preview`: the keys are hashed, so an unbounded
    // one only buys the caller hashing work on a multi-megabyte string.
    if scope.len() > MAX_CACHE_KEY_LEN || path.len() > MAX_CACHE_KEY_LEN {
        return Err("preview cache key too long".into());
    }
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
    // A preview is a downscaled JPEG. Without a cap, a caller that reaches this
    // command can fill the disk one oversized "preview" at a time — eviction
    // only runs against the configured limit, which it would blow past in a
    // single write.
    if bytes_b64.len() > MAX_PREVIEW_B64_LEN {
        return Err("preview too large to cache".into());
    }
    if scope.len() > MAX_CACHE_KEY_LEN || path.len() > MAX_CACHE_KEY_LEN {
        return Err("preview cache key too long".into());
    }
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, bytes_b64)
        .map_err(|e| format!("invalid preview cache payload: {e}"))?;
    if bytes.len() > MAX_PREVIEW_BYTES {
        return Err("preview too large to cache".into());
    }
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
        use crate::state::{merge_session_tokens, session_ready_for_sync, WebviewSessionTokens};

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
