use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use anyhow::Result;
use sarca_sync::{Binding, BindingMode, KeepBothPrompt, SarcaApi, SyncEngine, SyncEngineConfig};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Url};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ServerConfig {
    pub base_url: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub email_verified: bool,
}

impl ServerConfig {
    pub fn is_connected(&self) -> bool {
        !self.base_url.trim().is_empty() && !self.access_token.trim().is_empty()
    }

    pub fn app_url(&self) -> Result<Url> {
        let base = self.base_url.trim().trim_end_matches('/');
        Url::parse(&format!("{base}/")).map_err(|e| anyhow::anyhow!(e))
    }
}

#[derive(Clone)]
pub struct SessionInject {
    pub access_token: String,
    pub refresh_token: String,
    pub email: String,
    pub email_verified: bool,
}

impl From<&ServerConfig> for SessionInject {
    fn from(cfg: &ServerConfig) -> Self {
        Self {
            access_token: cfg.access_token.clone(),
            refresh_token: cfg.refresh_token.clone(),
            email: cfg.email.clone(),
            email_verified: cfg.email_verified,
        }
    }
}

impl SessionInject {
    pub fn eval_script(&self) -> String {
        let access = serde_json::to_string(&self.access_token).unwrap_or_else(|_| "\"\"".into());
        let refresh = serde_json::to_string(&self.refresh_token).unwrap_or_else(|_| "\"\"".into());
        let user = serde_json::to_string(&serde_json::json!({
            "email": self.email,
            "email_verified": self.email_verified,
        }))
        .unwrap_or_else(|_| "{}".into());
        format!(
            r#"(function(){{
  try {{
    localStorage.setItem('access_token', {access});
    localStorage.setItem('refresh_token', {refresh});
    localStorage.setItem('user', {user});
    localStorage.setItem('sarca_native', '1');
    window.__SARCA_NATIVE__ = 1;
    try {{ window.dispatchEvent(new Event('sarca-native')); }} catch (_) {{}}
    if (sessionStorage.getItem('__sarca_native_session') !== '1') {{
      sessionStorage.setItem('__sarca_native_session', '1');
      var u = new URL(location.href);
      u.pathname = '/';
      u.search = '';
      u.hash = '';
      u.searchParams.set('__sarca_native', '1');
      location.replace(u.toString());
    }}
  }} catch (e) {{ console.error('sarca session inject', e); }}
}})();"#
        )
    }
}

/// Runs before page scripts when possible (Tauri plugin init script).
/// Marks the origin as native so server UI can show Sync without waiting for
/// post-load eval (which is often too late / racy on Android WebView).
pub const NATIVE_MARK_JS: &str = r#"(function(){
  try {
    localStorage.setItem('sarca_native', '1');
    window.__SARCA_NATIVE__ = 1;
  } catch (e) {}
})();"#;

/// JS that opens local Sync settings from a remote-origin page.
/// Prefer invoke (local shell) or custom scheme (cross-scheme nav Tauri cancels).
/// Query-param fallback keeps the current path so a missed intercept does not
/// land on `/sync.html` on the Sarca server (SPA 404).
pub const OPEN_SYNC_JS: &str = r#"function __sarcaOpenSyncSettings(){
  try {
    if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
      window.__TAURI_INTERNALS__.invoke('open_sync_settings');
      return;
    }
  } catch (_) {}
  try {
    location.assign('sarca-sync://open');
    return;
  } catch (_) {}
  try {
    var u = new URL(location.href);
    u.searchParams.set('__sarca_open_sync', '1');
    location.assign(u.toString());
  } catch (_) {}
}
window.__sarcaOpenSyncSettings = __sarcaOpenSyncSettings;"#;

/// Injected on every remote page load when the webview is past the connect shell.
/// Always marks native (do not require a prior flag) and adds a visible Sync FAB.
pub fn native_chrome_js() -> String {
    format!(
        r#"(function(){{
  try {{
    localStorage.setItem('sarca_native', '1');
    window.__SARCA_NATIVE__ = 1;
    try {{ window.dispatchEvent(new Event('sarca-native')); }} catch (_) {{}}
    {open_sync}
    if (document.getElementById('sarca-native-sync-fab')) return;
    var btn = document.createElement('button');
    btn.id = 'sarca-native-sync-fab';
    btn.type = 'button';
    btn.textContent = 'Sync';
    btn.title = 'Media auto-upload and folder sync';
    btn.setAttribute('aria-label', 'Open Sync settings');
    btn.style.cssText = [
      'position:fixed',
      'z-index:2147483000',
      'right:max(12px,env(safe-area-inset-right))',
      'bottom:max(12px,env(safe-area-inset-bottom))',
      'padding:10px 14px',
      'border:none',
      'border-radius:10px',
      'background:#005a9e',
      'color:#fff',
      'font:600 14px/1.2 "Segoe UI",system-ui,sans-serif',
      'box-shadow:0 4px 14px rgba(0,0,0,.25)',
      'cursor:pointer'
    ].join(';');
    btn.onclick = function () {{ __sarcaOpenSyncSettings(); }};
    (document.body || document.documentElement).appendChild(btn);
  }} catch (e) {{}}
}})();"#,
        open_sync = OPEN_SYNC_JS
    )
}

pub struct AppSyncState {
    pub engine: Arc<SyncEngine>,
    pub server: Arc<Mutex<ServerConfig>>,
    pub pending_inject: Arc<StdMutex<Option<SessionInject>>>,
    pub shell_url: Arc<StdMutex<Option<Url>>>,
    data_dir: PathBuf,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl AppSyncState {
    pub fn new(app: &AppHandle) -> Result<Self> {
        let data_dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from(".").join("sarca-client-data"));
        fs::create_dir_all(&data_dir)?;

        let server = load_server_config(&data_dir);
        let api = Arc::new(RwLock::new(SarcaApi::new(
            &server.base_url,
            &server.access_token,
        )));
        let config = SyncEngineConfig {
            poll_interval: Duration::from_secs(30),
            api,
            data_dir: data_dir.clone(),
        };
        let engine = Arc::new(SyncEngine::open(config, Arc::new(KeepBothPrompt))?);
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);

        Ok(Self {
            engine,
            server: Arc::new(Mutex::new(server)),
            pending_inject: Arc::new(StdMutex::new(None)),
            shell_url: Arc::new(StdMutex::new(None)),
            data_dir,
            shutdown_tx,
        })
    }

    pub fn start_background_loop(&self) {
        let engine = self.engine.clone();
        let rx = self.shutdown_tx.subscribe();
        tauri::async_runtime::spawn(async move {
            engine.run_loop(rx).await;
        });
    }

    pub fn server_config_path(&self) -> PathBuf {
        self.data_dir.join("server.json")
    }

    pub async fn save_server(&self, cfg: &ServerConfig) -> Result<()> {
        let json = serde_json::to_string_pretty(cfg)?;
        fs::write(self.server_config_path(), json)?;
        *self.server.lock().await = cfg.clone();
        self.engine
            .set_credentials(cfg.base_url.clone(), cfg.access_token.clone())
            .await;
        Ok(())
    }

    pub fn queue_inject(&self, inject: SessionInject) {
        if let Ok(mut guard) = self.pending_inject.lock() {
            *guard = Some(inject);
        }
    }

    pub fn take_inject(&self) -> Option<SessionInject> {
        self.pending_inject.lock().ok().and_then(|mut g| g.take())
    }

    /// Remember the local connect-shell origin once. Never overwrite with a later
    /// URL — a Sarca server on localhost must not replace the Tauri asset origin.
    pub fn remember_shell_url(&self, url: Url) {
        if !is_shell_url(&url) {
            return;
        }
        if let Ok(mut guard) = self.shell_url.lock() {
            if guard.is_none() {
                *guard = Some(url);
            }
        }
    }

    pub fn shell_url(&self) -> Option<Url> {
        self.shell_url.lock().ok().and_then(|g| g.clone())
    }
}

/// True only for the Tauri-bundled connect shell / assets — not for a Sarca
/// server that happens to run on localhost (that used to make Sync open
/// `{server}/sync.html` and hit the SPA 404 page).
pub fn is_shell_url(url: &Url) -> bool {
    match url.scheme() {
        "tauri" | "asset" => true,
        "http" | "https" => match url.host_str() {
            Some("tauri.localhost") | Some("asset.localhost") => true,
            // Vite `devUrl` in tauri.conf.json (port 1420) only.
            Some("localhost") | Some("127.0.0.1") => url.port() == Some(1420),
            _ => false,
        },
        _ => false,
    }
}

/// Local `sync.html` URL derived from the remembered shell origin.
pub fn sync_settings_url(shell: &Url) -> Result<Url, String> {
    let mut base = shell.clone();
    base.set_path("/");
    base.set_query(None);
    base.set_fragment(None);
    base.join("sync.html").map_err(|e| e.to_string())
}

pub fn load_server_config(data_dir: &PathBuf) -> ServerConfig {
    let path = data_dir.join("server.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn parse_mode(mode: &str) -> BindingMode {
    match mode {
        "auto_upload" => BindingMode::AutoUpload,
        _ => BindingMode::Sync,
    }
}

pub fn new_binding(
    storage_id: &str,
    remote_root: String,
    local_path: String,
    mode: &str,
) -> Result<Binding> {
    Ok(Binding {
        id: Uuid::new_v4().to_string(),
        storage_id: Uuid::parse_str(storage_id)?,
        remote_root,
        local_path,
        mode: parse_mode(mode),
        enabled: true,
    })
}

pub fn navigate_to_server(app: &AppHandle, cfg: &ServerConfig) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window missing".to_string())?;
    let state = app.state::<AppSyncState>();
    state.queue_inject(SessionInject::from(cfg));
    let url = cfg.app_url().map_err(|e| e.to_string())?;
    window.navigate(url).map_err(|e| e.to_string())
}

pub fn navigate_to_shell(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window missing".to_string())?;
    let state = app.state::<AppSyncState>();
    let url = state
        .shell_url()
        .unwrap_or_else(|| Url::parse("tauri://localhost").expect("valid shell url"));
    window.navigate(url).map_err(|e| e.to_string())
}

pub fn navigate_to_sync_settings(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window missing".to_string())?;
    let state = app.state::<AppSyncState>();
    let base = state
        .shell_url()
        .unwrap_or_else(|| Url::parse("tauri://localhost").expect("valid shell url"));
    state.remember_shell_url(base.clone());
    let sync_url = sync_settings_url(&base)?;
    window.navigate(sync_url).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_url_accepts_tauri_origins_only() {
        assert!(is_shell_url(
            &Url::parse("tauri://localhost/index.html").unwrap()
        ));
        assert!(is_shell_url(
            &Url::parse("https://tauri.localhost/").unwrap()
        ));
        assert!(is_shell_url(
            &Url::parse("http://localhost:1420/").unwrap()
        ));
        // Sarca server on localhost must NOT be treated as the client shell.
        assert!(!is_shell_url(
            &Url::parse("http://127.0.0.1:8080/storages").unwrap()
        ));
        assert!(!is_shell_url(
            &Url::parse("https://localhost/sync.html").unwrap()
        ));
        assert!(!is_shell_url(
            &Url::parse("https://example.com/").unwrap()
        ));
    }

    #[test]
    fn sync_settings_url_is_local_sync_html() {
        let u = sync_settings_url(&Url::parse("tauri://localhost/index.html").unwrap()).unwrap();
        assert_eq!(u.as_str(), "tauri://localhost/sync.html");
        let u = sync_settings_url(&Url::parse("https://tauri.localhost/foo").unwrap()).unwrap();
        assert_eq!(u.as_str(), "https://tauri.localhost/sync.html");
    }
}
