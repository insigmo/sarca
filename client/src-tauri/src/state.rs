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

/// JS bridge for remote-origin pages: invoke native commands + open Settings → Sync.
///
/// Order (see also `tests::open_sync_js_fallback_order`):
/// 1. `__TAURI_INTERNALS__.invoke` — reliable once `remote.urls` match (incl. `:port`)
/// 2. `fetch` `sarca-ipc` custom protocol (bypasses ACL; often blocked on WebKitGTK)
/// 3. Cancelled navigation / iframe to `https://sarca.ipc` (bypasses ACL)
pub const OPEN_SYNC_JS: &str = r#"
(function(){
  if (!window.__sarcaIpcPending) window.__sarcaIpcPending = {};
  if (!window.__sarcaIpcSeq) window.__sarcaIpcSeq = 0;
  window.__sarcaIpcResolve = function(id, ok, value){
    var pending = window.__sarcaIpcPending && window.__sarcaIpcPending[id];
    if (!pending) return;
    delete window.__sarcaIpcPending[id];
    if (ok) pending.resolve(value);
    else pending.reject(new Error(typeof value === 'string' ? value : (value && value.message) || 'Native command failed'));
  };
  function __sarcaIpcEndpoints(){
    // Windows/Android map custom schemes to http(s)://<scheme>.localhost/
    // macOS/iOS/Linux use <scheme>://localhost/ — but http://sarca-ipc.localhost
    // often works when the raw scheme fetch is blocked (mixed content / WebKit).
    var ua = navigator.userAgent || '';
    var winOrDroid = /Windows/i.test(ua) || /Android/i.test(ua);
    if (winOrDroid) {
      return [
        'http://sarca-ipc.localhost/invoke',
        'https://sarca-ipc.localhost/invoke',
        'sarca-ipc://localhost/invoke'
      ];
    }
    return [
      'http://sarca-ipc.localhost/invoke',
      'sarca-ipc://localhost/invoke',
      'https://sarca-ipc.localhost/invoke'
    ];
  }
  function __sarcaFetchInvoke(cmd, args){
    var body = JSON.stringify({ cmd: cmd, args: args || {} });
    var endpoints = __sarcaIpcEndpoints();
    var lastErr = null;
    function tryAt(i){
      if (i >= endpoints.length) {
        return Promise.reject(lastErr || new Error('Native bridge unavailable'));
      }
      return fetch(endpoints[i], {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: body,
        credentials: 'omit',
        cache: 'no-store'
      }).then(function(res){
        return res.json().then(function(data){
          if (data && data.ok) return data.value;
          throw new Error((data && data.error) || ('Native command failed (' + res.status + ')'));
        }, function(){
          throw new Error('Native bridge bad response');
        });
      }).catch(function(err){
        lastErr = err;
        return tryAt(i + 1);
      });
    }
    return tryAt(0);
  }
  function __sarcaNavInvoke(cmd, args){
    return new Promise(function(resolve, reject){
      var id = 'r' + String(++window.__sarcaIpcSeq) + '_' + Date.now().toString(36);
      window.__sarcaIpcPending[id] = { resolve: resolve, reject: reject };
      var raw = encodeURIComponent(JSON.stringify({ id: id, cmd: cmd, args: args || {} }));
      var primary = 'https://sarca.ipc/__invoke__?p=' + raw;
      var secondary = 'sarca-ipc://localhost/__invoke__?p=' + raw;
      function tryIframe(src){
        var iframe = document.createElement('iframe');
        iframe.setAttribute('aria-hidden', 'true');
        iframe.style.cssText = 'display:none;width:0;height:0;border:0;position:absolute;left:-9999px';
        iframe.src = src;
        (document.documentElement || document.body).appendChild(iframe);
        setTimeout(function(){ try { iframe.remove(); } catch (_) {} }, 4000);
      }
      try { tryIframe(primary); } catch (_) {}
      try { tryIframe(secondary); } catch (_) {}
      // Top-level assign is cancelled by Rust on_navigation (more reliable than
      // iframes on some WebKitGTK builds). Only used if still pending shortly.
      setTimeout(function(){
        if (!window.__sarcaIpcPending[id]) return;
        try { window.location.assign(primary); } catch (_) {}
      }, 50);
      setTimeout(function(){
        if (!window.__sarcaIpcPending[id]) return;
        delete window.__sarcaIpcPending[id];
        reject(new Error('Native bridge timeout'));
      }, 90000);
    });
  }
  function __sarcaCombineErr(errs){
    var parts = [];
    for (var i = 0; i < errs.length; i++) {
      var e = errs[i];
      if (!e) continue;
      var m = (e && e.message) ? e.message : String(e);
      if (m && parts.indexOf(m) < 0) parts.push(m);
    }
    return new Error(parts.join(' | ') || 'Native bridge unavailable');
  }
  function __sarcaInvoke(cmd, args){
    function viaTauri(){
      try {
        if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
          return window.__TAURI_INTERNALS__.invoke(cmd, args || {});
        }
      } catch (_) {}
      return Promise.reject(new Error('Tauri invoke unavailable'));
    }
    // Prefer Tauri invoke (remote.urls must include host:port patterns).
    // Custom-protocol fetch often yields TypeError: Load failed on WebKitGTK.
    // Navigation IPC bypasses ACL when both prior paths fail.
    return viaTauri().catch(function(tauriErr){
      return __sarcaFetchInvoke(cmd, args).catch(function(fetchErr){
        return __sarcaNavInvoke(cmd, args).catch(function(navErr){
          throw __sarcaCombineErr([tauriErr, fetchErr, navErr]);
        });
      });
    });
  }
  window.__sarcaInvoke = __sarcaInvoke;
  function __sarcaOpenSyncSettings(){
    try {
      var u = new URL(location.href);
      u.searchParams.set('__sarca_open_settings', 'sync');
      history.replaceState(null, '', u.pathname + u.search + u.hash);
      window.dispatchEvent(new CustomEvent('sarca-open-settings', { detail: { tab: 'sync' } }));
      return;
    } catch (_) {}
    try { location.assign('sarca-sync://open'); } catch (_) {}
  }
  window.__sarcaOpenSyncSettings = __sarcaOpenSyncSettings;
})();
"#;

/// Injected on every remote page load when the webview is past the connect shell.
/// Marks native and installs the invoke / open-settings bridge (never creates a Sync FAB).
pub fn native_chrome_js() -> String {
    format!(
        r#"(function(){{
  try {{
    localStorage.setItem('sarca_native', '1');
    window.__SARCA_NATIVE__ = 1;
    try {{ window.dispatchEvent(new Event('sarca-native')); }} catch (_) {{}}
    {open_sync}
    // Strip legacy FAB from older client builds if still present in DOM.
    try {{
      var fab = document.getElementById('sarca-native-sync-fab');
      if (fab && fab.parentNode) fab.parentNode.removeChild(fab);
      document.querySelectorAll('[data-sarca-sync-fab],button.sarca-native-sync-fab').forEach(function(el){{
        if (el && el.parentNode) el.parentNode.removeChild(el);
      }});
    }} catch (_) {{}}
  }} catch (e) {{}}
}})();"#,
        open_sync = OPEN_SYNC_JS
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientPrefs {
    #[serde(default = "default_true")]
    pub wifi_only: bool,
    #[serde(default = "default_true")]
    pub background_sync: bool,
    #[serde(default)]
    pub app_lock_enabled: bool,
    #[serde(default)]
    pub app_lock_pin: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for ClientPrefs {
    fn default() -> Self {
        Self {
            wifi_only: true,
            background_sync: true,
            app_lock_enabled: false,
            app_lock_pin: None,
        }
    }
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

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    pub fn start_background_loop(&self) {
        let engine = self.engine.clone();
        let data_dir = self.data_dir.clone();
        let mut rx = self.shutdown_tx.subscribe();
        tauri::async_runtime::spawn(async move {
            loop {
                let prefs = fs::read_to_string(data_dir.join("client_prefs.json"))
                    .ok()
                    .and_then(|s| serde_json::from_str::<ClientPrefs>(&s).ok())
                    .unwrap_or_default();
                if prefs.background_sync {
                    let allow_auto = crate::commands::allow_auto_upload(&prefs);
                    if let Err(e) = engine
                        .tick_filtered(|b| {
                            if matches!(b.mode, BindingMode::AutoUpload) && !allow_auto {
                                return false;
                            }
                            true
                        })
                        .await
                    {
                        tracing::warn!(error = %e, "sync tick error");
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(30)) => {},
                    _ = rx.changed() => {
                        if *rx.borrow() {
                            break;
                        }
                    }
                }
            }
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
    let cfg = tauri::async_runtime::block_on(state.server.lock()).clone();
    if cfg.is_connected() {
        state.queue_inject(SessionInject::from(&cfg));
        let mut url = cfg.app_url().map_err(|e| e.to_string())?;
        // Open in-app Settings → Sync (not sync.html as primary UI).
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("__sarca_open_settings", "sync");
        }
        window.navigate(url).map_err(|e| e.to_string())
    } else {
        navigate_to_shell(app)
    }
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

    #[test]
    fn open_sync_js_fallback_order_prefers_tauri_then_fetch_then_nav() {
        // Guard against regressing to "fetch first" which surfaces
        // TypeError: Load failed on WebKitGTK when custom protocol is blocked.
        let js = OPEN_SYNC_JS;
        let invoke = js
            .find("function __sarcaInvoke")
            .expect("__sarcaInvoke");
        let body = &js[invoke..];
        let via_tauri_call = body.find("return viaTauri()").expect("prefer viaTauri");
        let fetch_call = body
            .find("__sarcaFetchInvoke")
            .expect("fetch fallback");
        let nav_call = body.find("__sarcaNavInvoke").expect("nav fallback");
        assert!(
            via_tauri_call < fetch_call && fetch_call < nav_call,
            "expected viaTauri → fetch → nav order inside __sarcaInvoke"
        );
        assert!(
            js.contains("__sarcaCombineErr"),
            "must combine errors instead of rethrowing only Load failed"
        );
        assert!(js.contains("viaTauri"), "viaTauri helper must exist");
    }
}
