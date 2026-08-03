use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use anyhow::Result;
#[cfg(not(target_os = "android"))]
use sarca_sync::FsMediaSource;
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

/// Decode a value written by the UI's `createLocalStore` (`JSON.stringify`) or by
/// a raw `localStorage.setItem`. Website login stores `"\"jwt...\""` (with quotes);
/// reading that without parse was sending quoted JWTs to Sync → 401 / "Session expired".
pub fn normalize_stored_token(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    if t.starts_with('"') {
        if let Ok(s) = serde_json::from_str::<String>(t) {
            return s.trim().to_owned();
        }
    }
    t.to_owned()
}

/// Tokens read from the remote webview's localStorage (after JSON decode).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct WebviewSessionTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: Option<bool>,
}

impl WebviewSessionTokens {
    /// Parse raw localStorage strings (createLocalStore JSON encoding).
    /// Production pulls via `READ_WEBVIEW_SESSION_JS` + `read_webview_session`;
    /// this mirrors that decoding for unit tests.
    #[cfg(test)]
    pub fn from_local_storage_raw(
        access_raw: Option<&str>,
        refresh_raw: Option<&str>,
        user_raw: Option<&str>,
    ) -> Option<Self> {
        let access = normalize_stored_token(access_raw.unwrap_or(""));
        if access.is_empty() {
            return None;
        }
        let refresh = refresh_raw
            .map(normalize_stored_token)
            .filter(|s| !s.is_empty());
        let mut email = None;
        let mut email_verified = None;
        if let Some(raw) = user_raw {
            let parsed: Option<serde_json::Value> = serde_json::from_str(raw.trim()).ok();
            if let Some(user) = parsed {
                email = user
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned());
                email_verified = user.get("email_verified").and_then(|v| v.as_bool());
            }
        }
        Some(Self {
            access_token: access,
            refresh_token: refresh,
            email,
            email_verified,
        })
    }

    pub fn has_access(&self) -> bool {
        !self.access_token.trim().is_empty()
    }
}

/// True when Sync may proceed: native has tokens, or the webview still has them
/// (caller should pull/apply webview tokens before treating as logged out).
pub fn session_ready_for_sync(native_connected: bool, webview_has_access: bool) -> bool {
    native_connected || webview_has_access
}

/// Merge tokens from the logged-in webview into native `ServerConfig`.
/// Used so Sync API calls (create_folder, etc.) share the same session as the UI.
pub fn merge_session_tokens(
    cfg: &mut ServerConfig,
    access_token: &str,
    refresh_token: Option<&str>,
    email: Option<&str>,
    email_verified: Option<bool>,
) -> Result<(), String> {
    let access = normalize_stored_token(access_token);
    if access.is_empty() {
        return Err(
            "Not authenticated — missing access token. Sign in again so Sync can use your session."
                .into(),
        );
    }
    if cfg.base_url.trim().is_empty() {
        return Err("Not connected".into());
    }
    cfg.access_token = access;
    if let Some(r) = refresh_token {
        let r = normalize_stored_token(r);
        if !r.is_empty() {
            cfg.refresh_token = r;
        }
    }
    if let Some(e) = email {
        let e = e.trim();
        if !e.is_empty() {
            cfg.email = e.to_owned();
        }
    }
    if let Some(v) = email_verified {
        cfg.email_verified = v;
    }
    Ok(())
}

/// JS snippet: return `{access_token, refresh_token?, email?, email_verified?}` from
/// localStorage, matching `createLocalStore` JSON encoding. Used by Rust `eval_with_callback`.
pub const READ_WEBVIEW_SESSION_JS: &str = r#"(function(){
  function parseLs(key){
    try {
      var raw = localStorage.getItem(key);
      if (raw == null || raw === '') return null;
      try { return JSON.parse(raw); } catch (_) { return raw; }
    } catch (_) { return null; }
  }
  try {
    var at = parseLs('access_token');
    if (at == null || !String(at).trim()) return null;
    var out = { access_token: String(at).trim() };
    var rt = parseLs('refresh_token');
    if (rt != null && String(rt).trim()) out.refresh_token = String(rt).trim();
    var user = parseLs('user');
    if (user && typeof user === 'object') {
      if (user.email) out.email = String(user.email);
      if (typeof user.email_verified === 'boolean') out.email_verified = user.email_verified;
    }
    return out;
  } catch (_) { return null; }
})()"#;

/// Actively read session tokens from the main webview localStorage.
/// Returns `None` when the webview is missing, eval fails, or no access token is stored.
pub async fn read_webview_session(app: &AppHandle) -> Option<WebviewSessionTokens> {
    let window = app.get_webview_window("main")?;
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    // eval_with_callback requires `Fn` (not FnOnce), so wrap the oneshot sender.
    let tx = StdMutex::new(Some(tx));
    window
        .eval_with_callback(READ_WEBVIEW_SESSION_JS, move |result| {
            if let Ok(mut guard) = tx.lock() {
                if let Some(sender) = guard.take() {
                    let _ = sender.send(result);
                }
            }
        })
        .ok()?;
    let raw = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .ok()?
        .ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" || trimmed == "undefined" {
        return None;
    }
    // Callback receives a JSON-serialized evaluation result (string of an object or null).
    let parsed: WebviewSessionTokens = serde_json::from_str(trimmed).ok()?;
    if !parsed.has_access() {
        return None;
    }
    Some(WebviewSessionTokens {
        access_token: normalize_stored_token(&parsed.access_token),
        refresh_token: parsed
            .refresh_token
            .map(|r| normalize_stored_token(&r))
            .filter(|s| !s.is_empty()),
        email: parsed.email,
        email_verified: parsed.email_verified,
    })
}

#[derive(Clone)]
pub struct SessionInject {
    pub access_token: String,
    pub refresh_token: String,
    pub email: String,
    pub email_verified: bool,
    /// Origin the tokens belong to (`http://host:port`). The script refuses to
    /// run anywhere else: a page-load event can name the pending remote URL
    /// while the connect shell is still the live document, and injecting there
    /// would rewrite the shell's location and strand the app on it.
    pub origin: String,
}

impl From<&ServerConfig> for SessionInject {
    fn from(cfg: &ServerConfig) -> Self {
        Self {
            access_token: cfg.access_token.clone(),
            refresh_token: cfg.refresh_token.clone(),
            email: cfg.email.clone(),
            email_verified: cfg.email_verified,
            origin: cfg
                .app_url()
                .map(|u| u.origin().ascii_serialization())
                .unwrap_or_default(),
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
        let origin = serde_json::to_string(&self.origin).unwrap_or_else(|_| "\"\"".into());
        // JSON.stringify so values match createLocalStore (website login).
        format!(
            r#"(function(){{
  try {{
    var want = {origin};
    if (want && location.origin !== want) return;
    localStorage.setItem('access_token', JSON.stringify({access}));
    localStorage.setItem('refresh_token', JSON.stringify({refresh}));
    localStorage.setItem('user', JSON.stringify({user}));
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
    var command = [];
    var bridge = [];
    for (var i = 0; i < errs.length; i++) {
      var e = errs[i];
      if (!e) continue;
      var m = (e && e.message) ? e.message : String(e);
      if (!m) continue;
      if (__sarcaIsBridgeError(e)) {
        if (bridge.indexOf(m) < 0) bridge.push(m);
      } else {
        if (command.indexOf(m) < 0) command.push(m);
      }
    }
    // Prefer real command/API errors over transport noise ("Load failed").
    if (command.length) return new Error(command.join(' | '));
    return new Error(bridge.join(' | ') || 'Native bridge unavailable');
  }
  function __sarcaIsBridgeError(err){
    var m = ((err && err.message) ? err.message : String(err || '')).toLowerCase();
    if (!m) return true;
    return (
      m.indexOf('not allowed') >= 0 ||
      m.indexOf('denied by acl') >= 0 ||
      m.indexOf('command') >= 0 && m.indexOf('not allowed') >= 0 ||
      m.indexOf('unavailable') >= 0 ||
      m.indexOf('tauri invoke unavailable') >= 0 ||
      m.indexOf('native bridge') >= 0 ||
      m.indexOf('load failed') >= 0 ||
      m.indexOf('failed to fetch') >= 0 ||
      m.indexOf('networkerror') >= 0 ||
      m.indexOf('network error') >= 0 ||
      m.indexOf('bridge timeout') >= 0 ||
      m.indexOf('unknown native command') >= 0
    );
  }
  function __sarcaParseLs(key){
    try {
      var raw = localStorage.getItem(key);
      if (raw == null || raw === '') return null;
      // createLocalStore writes JSON.stringify(value); also accept raw inject.
      try { return JSON.parse(raw); } catch (_) { return raw; }
    } catch (_) { return null; }
  }
  function __sarcaReadSession(){
    try {
      var at = __sarcaParseLs('access_token');
      if (at == null || !String(at).trim()) return null;
      var out = { accessToken: String(at).trim() };
      var rt = __sarcaParseLs('refresh_token');
      if (rt != null && String(rt).trim()) out.refreshToken = String(rt).trim();
      var user = __sarcaParseLs('user');
      if (user && typeof user === 'object') {
        if (user.email) out.email = String(user.email);
        if (typeof user.email_verified === 'boolean') out.emailVerified = user.email_verified;
      }
      return out;
    } catch (_) {
      return null;
    }
  }
  function __sarcaInvoke(cmd, args){
    function invokeOnce(c, a){
      function viaTauri(){
        try {
          if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
            return window.__TAURI_INTERNALS__.invoke(c, a || {});
          }
        } catch (_) {}
        return Promise.reject(new Error('Tauri invoke unavailable'));
      }
      // Prefer Tauri invoke (remote.urls must include host:port patterns).
      // Custom-protocol fetch often yields TypeError: Load failed on WebKitGTK.
      // Navigation IPC bypasses ACL when both prior paths fail.
      // Do NOT fall through when Tauri already returned a command/API error
      // (e.g. create_folder 401) — that only appends "| Load failed" noise.
      return viaTauri().catch(function(tauriErr){
        if (!__sarcaIsBridgeError(tauriErr)) throw tauriErr;
        return __sarcaFetchInvoke(c, a).catch(function(fetchErr){
          if (!__sarcaIsBridgeError(fetchErr)) throw fetchErr;
          return __sarcaNavInvoke(c, a).catch(function(navErr){
            throw __sarcaCombineErr([tauriErr, fetchErr, navErr]);
          });
        });
      });
    }
    var payload = args || {};
    // Push the webview's live tokens into native state before Sync API commands
    // so create_folder / list_storages use the same session as the logged-in UI.
    if (cmd !== 'update_session' && cmd !== 'connect' && cmd !== 'disconnect') {
      var session = __sarcaReadSession();
      if (session) {
        return invokeOnce('update_session', session).catch(function(){
          // Best-effort; still attempt the original command.
        }).then(function(){
          return invokeOnce(cmd, payload);
        });
      }
    }
    return invokeOnce(cmd, payload);
  }
  // Preview disk cache must never block opening images. cache_put payloads are
  // large base64 JPEGs — navigation IPC puts them in a URL and hits length /
  // timeout limits on WebKitGTK, which used to surface as "Could not open this file".
  window.__sarcaInvoke = function(cmd, args){
    if (cmd === 'cache_get_preview') {
      return __sarcaInvoke(cmd, args).catch(function(){ return null; });
    }
    if (cmd === 'cache_put_preview') {
      function putViaTauri(){
        try {
          if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
            return window.__TAURI_INTERNALS__.invoke(cmd, args || {});
          }
        } catch (_) {}
        return Promise.reject(new Error('Tauri invoke unavailable'));
      }
      return putViaTauri().catch(function(tauriErr){
        if (!__sarcaIsBridgeError(tauriErr)) throw tauriErr;
        return __sarcaFetchInvoke(cmd, args || {});
      }).catch(function(){
        return null;
      });
    }
    return __sarcaInvoke(cmd, args);
  };
  // Guard WebKit RangeError from String.fromCharCode(...largeTypedArray) used by
  // older FileViewer preview→base64 encoding before cache_put_preview.
  (function(){
    var orig = String.fromCharCode;
    String.fromCharCode = function(){
      var n = arguments.length;
      if (n <= 8192) return orig.apply(null, arguments);
      var out = '';
      for (var i = 0; i < n; i += 8192) {
        var end = i + 8192 < n ? i + 8192 : n;
        var chunk = Array.prototype.slice.call(arguments, i, end);
        out += orig.apply(null, chunk);
      }
      return out;
    };
  })();
  // After website login, push tokens into native Sync state (and keep them fresh).
  // Shell Connect no longer POSTs /api/auth/login — tokens appear in localStorage.
  (function __sarcaWatchSession(){
    var last = '';
    function push(){
      try {
        var session = __sarcaReadSession();
        if (!session) { last = ''; return; }
        var key = session.accessToken + '|' + (session.refreshToken || '') + '|' + (session.email || '');
        if (key === last) return;
        last = key;
        __sarcaInvoke('update_session', session).catch(function(){});
      } catch (_) {}
    }
    push();
    setInterval(push, 1500);
    try {
      var _setItem = localStorage.setItem.bind(localStorage);
      localStorage.setItem = function(k, v){
        _setItem(k, v);
        if (k === 'access_token' || k === 'refresh_token' || k === 'user') {
          setTimeout(push, 0);
        }
      };
    } catch (_) {}
  })();
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

/// On-disk client preferences.
///
/// The app-lock PIN is stored as a salted, iterated hash. It used to be kept in
/// clear and handed back by `get_client_prefs`, so anything that reached the
/// IPC bridge could read the PIN and walk straight past the lock screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientPrefs {
    #[serde(default = "default_true")]
    pub wifi_only: bool,
    #[serde(default)]
    pub app_lock_enabled: bool,
    #[serde(default)]
    pub app_lock_pin_salt: Option<String>,
    #[serde(default)]
    pub app_lock_pin_hash: Option<String>,
    /// Legacy plaintext PIN from before hashing. Read once on load, converted,
    /// and dropped; never written back. The rename is what actually picks it
    /// up: old prefs files spell the key `app_lock_pin`.
    #[serde(rename = "app_lock_pin", default, skip_serializing)]
    pub legacy_app_lock_pin: Option<String>,
    #[serde(default = "default_true")]
    pub enable_logs: bool,
    #[serde(default = "default_cache_limit_bytes")]
    pub cache_limit_bytes: u64,
}

/// What the WebView is allowed to see and send. No hash, no salt, and the PIN
/// field is write-only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientPrefsDto {
    #[serde(default = "default_true")]
    pub wifi_only: bool,
    #[serde(default)]
    pub app_lock_enabled: bool,
    /// True when a PIN is set. Lets the UI show "change PIN" vs "set PIN"
    /// without ever learning the PIN itself.
    #[serde(default, skip_deserializing)]
    pub app_lock_pin_set: bool,
    /// Write-only: present to (re)define the PIN, never serialized back out.
    #[serde(default, skip_serializing)]
    pub app_lock_pin: Option<String>,
    /// Write-only: the PIN currently on file. Required to change or remove an
    /// existing PIN, so reaching `set_client_prefs` is not by itself enough to
    /// switch the app lock off.
    #[serde(default, skip_serializing)]
    pub current_app_lock_pin: Option<String>,
    #[serde(default = "default_true")]
    pub enable_logs: bool,
    #[serde(default = "default_cache_limit_bytes")]
    pub cache_limit_bytes: u64,
}

fn default_cache_limit_bytes() -> u64 {
    1_073_741_824
}

fn default_true() -> bool {
    true
}

/// Cap on cache size a caller may request: 64 GiB. Without it a hostile
/// `set_client_prefs` could disable eviction by asking for `u64::MAX`.
const MAX_CACHE_LIMIT_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MIN_CACHE_LIMIT_BYTES: u64 = 16 * 1024 * 1024;

/// PIN rules. Short enough to stay a PIN, long enough not to be guessable in
/// three taps; the length cap stops a caller from making us hash a huge string.
const MIN_PIN_LEN: usize = 4;
const MAX_PIN_LEN: usize = 64;
/// Iteration count for the PIN hash. A PIN has little entropy, so this only
/// buys time against someone who already reads `client_prefs.json`; the real
/// protection is the 0600 file mode and never returning the secret over IPC.
const PIN_HASH_ROUNDS: u32 = 120_000;

impl Default for ClientPrefs {
    fn default() -> Self {
        Self {
            wifi_only: true,
            app_lock_enabled: false,
            app_lock_pin_salt: None,
            app_lock_pin_hash: None,
            legacy_app_lock_pin: None,
            enable_logs: true,
            cache_limit_bytes: default_cache_limit_bytes(),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

fn hash_pin(pin: &str, salt_hex: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest.update(salt_hex.as_bytes());
    digest.update(pin.as_bytes());
    let mut out = digest.finalize();
    for _ in 1..PIN_HASH_ROUNDS {
        let mut next = Sha256::new();
        next.update(salt_hex.as_bytes());
        next.update(out);
        out = next.finalize();
    }
    hex(&out)
}

/// Compare without leaking the position of the first differing byte.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl ClientPrefs {
    /// Convert a legacy plaintext PIN into a hash. Returns true when the file
    /// needs rewriting (the plaintext PIN must not survive on disk).
    pub fn migrate_legacy_pin(&mut self) -> bool {
        let Some(pin) = self.legacy_app_lock_pin.take() else {
            return false;
        };
        if self.has_pin() {
            // Already migrated; the leftover key just needs dropping.
            return true;
        }
        if self.set_pin(pin.trim()).is_err() {
            // Empty or otherwise unusable legacy PIN. Leaving `app_lock_enabled`
            // on with nothing to verify against would make the gate wave
            // everyone through, so turn the lock off instead.
            self.clear_pin();
        }
        true
    }

    pub fn set_pin(&mut self, pin: &str) -> Result<(), String> {
        let pin = pin.trim();
        if pin.chars().count() < MIN_PIN_LEN {
            return Err(format!("PIN must be at least {MIN_PIN_LEN} characters"));
        }
        if pin.chars().count() > MAX_PIN_LEN {
            return Err(format!("PIN must be at most {MAX_PIN_LEN} characters"));
        }
        let salt = hex(Uuid::new_v4().as_bytes());
        self.app_lock_pin_hash = Some(hash_pin(pin, &salt));
        self.app_lock_pin_salt = Some(salt);
        Ok(())
    }

    pub fn clear_pin(&mut self) {
        self.app_lock_pin_hash = None;
        self.app_lock_pin_salt = None;
        self.app_lock_enabled = false;
    }

    pub fn has_pin(&self) -> bool {
        self.app_lock_pin_hash.is_some() && self.app_lock_pin_salt.is_some()
    }

    pub fn verify_pin(&self, candidate: &str) -> bool {
        let (Some(hash), Some(salt)) = (&self.app_lock_pin_hash, &self.app_lock_pin_salt) else {
            return false;
        };
        let candidate = candidate.trim();
        if candidate.is_empty() || candidate.chars().count() > MAX_PIN_LEN {
            return false;
        }
        constant_time_eq(hash, &hash_pin(candidate, salt))
    }

    pub fn to_dto(&self) -> ClientPrefsDto {
        ClientPrefsDto {
            wifi_only: self.wifi_only,
            app_lock_enabled: self.app_lock_enabled,
            app_lock_pin_set: self.has_pin(),
            app_lock_pin: None,
            current_app_lock_pin: None,
            enable_logs: self.enable_logs,
            cache_limit_bytes: self.cache_limit_bytes,
        }
    }

    /// Fold a request from the WebView into the stored prefs.
    ///
    /// The caller can never read the PIN, and it cannot change or remove one
    /// that is already set without presenting it: otherwise merely reaching
    /// `set_client_prefs` would be an app-lock bypass.
    pub fn apply_dto(&mut self, dto: ClientPrefsDto) -> Result<(), String> {
        let new_pin = dto.app_lock_pin.as_deref().map(str::trim).filter(|p| !p.is_empty());
        let wants_pin_change = new_pin.is_some();
        let wants_disable = self.app_lock_enabled && !dto.app_lock_enabled;

        if self.has_pin() && (wants_pin_change || wants_disable) {
            let current = dto.current_app_lock_pin.as_deref().unwrap_or_default();
            if !self.verify_pin(current) {
                return Err("Current PIN is incorrect".into());
            }
        }

        self.wifi_only = dto.wifi_only;
        self.enable_logs = dto.enable_logs;
        self.cache_limit_bytes = dto
            .cache_limit_bytes
            .clamp(MIN_CACHE_LIMIT_BYTES, MAX_CACHE_LIMIT_BYTES);

        if let Some(pin) = new_pin {
            self.set_pin(pin)?;
        }

        if dto.app_lock_enabled {
            if !self.has_pin() {
                return Err("Set a PIN before turning on the app lock".into());
            }
            self.app_lock_enabled = true;
        } else {
            self.clear_pin();
        }
        Ok(())
    }
}

pub struct AppSyncState {
    pub engine: Arc<SyncEngine>,
    pub server: Arc<Mutex<ServerConfig>>,
    pub pending_inject: Arc<StdMutex<Option<SessionInject>>>,
    pub shell_url: Arc<StdMutex<Option<Url>>>,
    /// Origin of the connected server, mirrored out of the async `server` mutex
    /// so the synchronous navigation hook can authorise an IPC call without
    /// blocking on a lock it cannot await.
    trusted_origin: Arc<StdMutex<Option<String>>>,
    data_dir: PathBuf,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// Bumped to interrupt the background-loop sleep and run a tick soon
    /// (app resume / foreground).
    wake_tx: tokio::sync::watch::Sender<u64>,
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
            #[cfg(target_os = "android")]
            media_source: Arc::new(crate::mediastore::AndroidDcimMediaSource::new(app.clone())),
            #[cfg(not(target_os = "android"))]
            media_source: Arc::new(FsMediaSource),
        };
        let engine = Arc::new(SyncEngine::open(config, Arc::new(KeepBothPrompt))?);
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        let (wake_tx, _) = tokio::sync::watch::channel(0u64);

        let trusted_origin = origin_of_base_url(&server.base_url);

        Ok(Self {
            engine,
            server: Arc::new(Mutex::new(server)),
            pending_inject: Arc::new(StdMutex::new(None)),
            shell_url: Arc::new(StdMutex::new(None)),
            trusted_origin: Arc::new(StdMutex::new(trusted_origin)),
            data_dir,
            shutdown_tx,
            wake_tx,
        })
    }

    /// Origin of the connected server, or `None` while disconnected.
    pub fn trusted_origin(&self) -> Option<String> {
        self.trusted_origin.lock().ok().and_then(|g| g.clone())
    }

    /// True when `url` may drive the native IPC bridge: the Tauri-bundled shell
    /// or the server the user actually connected to. Every other page — an
    /// iframe, an ad, a redirect, a `file://` document — is refused.
    pub fn is_trusted_ipc_url(&self, url: &Url) -> bool {
        if is_shell_url(url) {
            return true;
        }
        match (origin_string(url), self.trusted_origin()) {
            (Some(origin), Some(trusted)) => origin == trusted,
            _ => false,
        }
    }

    /// Same check against a serialized `Origin` header value.
    pub fn is_trusted_ipc_origin(&self, origin: &str) -> bool {
        let origin = origin.trim();
        // "null" is what a sandboxed iframe, a `data:` document or a redirected
        // cross-origin request sends. It is never trusted.
        if origin.is_empty() || origin.eq_ignore_ascii_case("null") {
            return false;
        }
        if SHELL_ORIGINS.iter().any(|o| o.eq_ignore_ascii_case(origin)) {
            return true;
        }
        match Url::parse(origin) {
            Ok(url) => self.is_trusted_ipc_url(&url),
            Err(_) => false,
        }
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Wake the background sync loop so the next tick runs without waiting
    /// out the full poll interval (used on Android/iOS resume).
    #[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
    pub fn request_sync_wake(&self) {
        let next = self.wake_tx.borrow().wrapping_add(1);
        let _ = self.wake_tx.send(next);
    }

    pub fn start_background_loop(&self) {
        let engine = self.engine.clone();
        let data_dir = self.data_dir.clone();
        let mut rx = self.shutdown_tx.subscribe();
        let mut wake_rx = self.wake_tx.subscribe();
        tauri::async_runtime::spawn(async move {
            // Let the Android activity / WebView paint before the first heavy
            // MediaStore scan; otherwise startup looks like a black screen ANR.
            #[cfg(any(target_os = "android", target_os = "ios"))]
            tokio::time::sleep(Duration::from_secs(3)).await;

            loop {
                // Pick up tokens written by the webview / sync_now without waiting
                // for another invoke.
                let server = load_server_config(&data_dir);
                let connected = server.is_connected();
                if connected {
                    engine
                        .set_credentials(server.base_url.clone(), server.access_token.clone())
                        .await;
                }
                let prefs = fs::read_to_string(data_dir.join("client_prefs.json"))
                    .ok()
                    .and_then(|s| serde_json::from_str::<ClientPrefs>(&s).ok())
                    .unwrap_or_default();
                // Never run discovery/upload while disconnected — MediaStore
                // listing still blocks the UI thread via the plugin bridge and
                // cannot upload without credentials anyway. Background sync
                // is otherwise always on — there is no user-facing toggle.
                if connected {
                    let allow_auto = crate::commands::allow_auto_upload(&prefs);
                    if let Err(e) = engine
                        .tick_filtered(|b| {
                            if b.mode.is_upload_only() && !allow_auto {
                                return false;
                            }
                            true
                        })
                        .await
                    {
                        tracing::warn!(error = %e, "sync tick error");
                        crate::client_log::write_line(&data_dir, &format!("sync tick error: {e}"));
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(30)) => {},
                    _ = wake_rx.changed() => {},
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
        write_private(&self.server_config_path(), json.as_bytes())?;
        if let Ok(mut guard) = self.trusted_origin.lock() {
            *guard = origin_of_base_url(&cfg.base_url);
        }
        *self.server.lock().await = cfg.clone();
        self.engine
            .set_credentials(cfg.base_url.clone(), cfg.access_token.clone())
            .await;
        Ok(())
    }

    /// Apply tokens from the remote webview (localStorage) into native sync state.
    pub async fn apply_webview_session(
        &self,
        access_token: String,
        refresh_token: Option<String>,
        email: Option<String>,
        email_verified: Option<bool>,
    ) -> Result<(), String> {
        let mut cfg = self.server.lock().await.clone();
        merge_session_tokens(
            &mut cfg,
            &access_token,
            refresh_token.as_deref(),
            email.as_deref(),
            email_verified,
        )?;
        self.save_server(&cfg).await.map_err(|e| e.to_string())
    }

    /// Best-effort: pull live tokens from the webview and merge into native state.
    /// Silent when webview has no tokens or eval is unavailable.
    pub async fn sync_session_from_webview(&self, app: &AppHandle) -> Option<WebviewSessionTokens> {
        let tokens = read_webview_session(app).await?;
        if let Err(e) = self
            .apply_webview_session(
                tokens.access_token.clone(),
                tokens.refresh_token.clone(),
                tokens.email.clone(),
                tokens.email_verified,
            )
            .await
        {
            tracing::debug!(error = %e, "webview session apply skipped");
            return None;
        }
        Some(tokens)
    }

    pub fn queue_inject(&self, inject: SessionInject) {
        if let Ok(mut guard) = self.pending_inject.lock() {
            *guard = Some(inject);
        }
    }

    pub fn take_inject(&self) -> Option<SessionInject> {
        self.pending_inject.lock().ok().and_then(|mut g| g.take())
    }

    /// Take the queued inject only when `url` is the origin it was queued for.
    /// A mismatch leaves it queued, so the real remote load still gets it.
    pub fn take_inject_for(&self, url: &Url) -> Option<SessionInject> {
        let mut guard = self.pending_inject.lock().ok()?;
        let origin = guard.as_ref().map(|i| i.origin.clone())?;
        if !origin.is_empty() && origin != url.origin().ascii_serialization() {
            return None;
        }
        guard.take()
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

    pub fn load_url_history(&self) -> Vec<String> {
        load_url_history(&self.data_dir)
    }

    pub fn record_url_history(&self, base_url: &str) -> Vec<String> {
        record_url_history(&self.data_dir, base_url)
    }
}

/// Origins of the Tauri-bundled shell, as the WebView serializes them in an
/// `Origin` header. Which one appears depends on the platform (custom scheme on
/// Linux/macOS, `*.localhost` on Windows/Android).
pub const SHELL_ORIGINS: &[&str] = &[
    "tauri://localhost",
    "asset://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
    "http://asset.localhost",
    "https://asset.localhost",
];

/// Serialized tuple origin (`scheme://host[:port]`).
///
/// `Url::origin()` yields an opaque origin for non-special schemes such as
/// `tauri:`, which would collapse every shell page to the same `null` value,
/// so the tuple is built by hand.
pub fn origin_string(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let scheme = url.scheme();
    Some(match url.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

/// Origin of a stored `base_url`, or `None` when it is empty or unparsable.
pub fn origin_of_base_url(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    Url::parse(trimmed).ok().as_ref().and_then(origin_string)
}

/// Write a file that only the current user can read.
///
/// `server.json` holds the access and refresh tokens and `client_prefs.json`
/// holds the app-lock secret; on a shared machine the default 0644 would hand
/// both to every other local account.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        // `mode` only applies when the file is created; fix up an existing one
        // that a previous version wrote with the default umask.
        fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        return Ok(());
    }

    #[cfg(not(unix))]
    {
        // Windows: the per-user AppData directory is already ACL'd to the user.
        fs::write(path, bytes)?;
        Ok(())
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
/// Kept for tests; production `navigate_to_sync_settings` opens in-app
/// Settings → Sync (or the connect shell when disconnected).
#[cfg(test)]
pub fn sync_settings_url(shell: &Url) -> Result<Url, String> {
    let mut base = shell.clone();
    base.set_path("/");
    base.set_query(None);
    base.set_fragment(None);
    base.join("sync.html").map_err(|e| e.to_string())
}

pub fn load_server_config(data_dir: &Path) -> ServerConfig {
    let path = data_dir.join("server.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

const URL_HISTORY_MAX: usize = 3;

fn url_history_path(data_dir: &Path) -> PathBuf {
    data_dir.join("url_history.json")
}

/// Most-recently-used server URLs shown on the Connect screen, newest first.
/// Kept separate from `ServerConfig` so `disconnect` (which resets the config)
/// does not wipe it.
pub fn load_url_history(data_dir: &Path) -> Vec<String> {
    fs::read_to_string(url_history_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Move `base_url` to the front of the history (deduped), capped at
/// `URL_HISTORY_MAX`, and persist. Returns the updated list.
pub fn record_url_history(data_dir: &Path, base_url: &str) -> Vec<String> {
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return load_url_history(data_dir);
    }
    let mut history = load_url_history(data_dir);
    history.retain(|u| u != &base_url);
    history.insert(0, base_url);
    history.truncate(URL_HISTORY_MAX);
    if let Ok(json) = serde_json::to_string_pretty(&history) {
        let _ = fs::write(url_history_path(data_dir), json);
    }
    history
}

pub fn parse_mode(mode: &str) -> BindingMode {
    match mode {
        "auto_upload" => BindingMode::AutoUpload,
        "folder_upload" => BindingMode::FolderUpload,
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
    // Only inject stored tokens when we have them. After URL-only Connect the
    // user signs in on the website; empty inject would wipe webview localStorage.
    if cfg.is_connected() {
        state.queue_inject(SessionInject::from(cfg));
    }
    // Give this one origin the Settings ACL right before we hand it the
    // WebView. Nothing is granted up front, so a page reached any other way
    // (redirect, iframe, a link the user clicked) has no capability at all.
    if let Some(origin) = origin_of_base_url(&cfg.base_url) {
        if let Err(e) = crate::remote_ipc::grant_remote_capability(app, &origin) {
            tracing::debug!(error = %e, origin, "remote capability already granted");
        }
    }
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

    fn prefs_with_pin(pin: &str) -> ClientPrefs {
        let mut prefs = ClientPrefs::default();
        prefs.set_pin(pin).expect("pin should be accepted");
        prefs.app_lock_enabled = true;
        prefs
    }

    fn dto_of(prefs: &ClientPrefs) -> ClientPrefsDto {
        prefs.to_dto()
    }

    #[test]
    fn pin_is_stored_salted_and_never_in_the_dto() {
        let prefs = prefs_with_pin("1234");
        let hash = prefs.app_lock_pin_hash.clone().unwrap();
        assert!(!hash.contains("1234"), "hash must not embed the PIN");
        assert!(prefs.app_lock_pin_salt.is_some(), "a salt must be stored");

        let dto = dto_of(&prefs);
        assert!(dto.app_lock_pin_set);
        assert!(dto.app_lock_pin.is_none());
        assert!(dto.current_app_lock_pin.is_none());

        let json = serde_json::to_string(&dto).unwrap();
        assert!(
            !json.contains("1234"),
            "the PIN must never reach the WebView: {json}"
        );
        assert!(!json.contains("app_lock_pin\":"), "no PIN field: {json}");
    }

    #[test]
    fn the_same_pin_hashes_differently_under_a_fresh_salt() {
        let a = prefs_with_pin("1234");
        let b = prefs_with_pin("1234");
        assert_ne!(a.app_lock_pin_salt, b.app_lock_pin_salt);
        assert_ne!(a.app_lock_pin_hash, b.app_lock_pin_hash);
        assert!(a.verify_pin("1234") && b.verify_pin("1234"));
    }

    #[test]
    fn verify_pin_rejects_wrong_empty_and_oversized_candidates() {
        let prefs = prefs_with_pin("1234");
        assert!(prefs.verify_pin("1234"));
        assert!(prefs.verify_pin(" 1234 "), "input is trimmed");
        assert!(!prefs.verify_pin("12345"));
        assert!(!prefs.verify_pin(""));
        assert!(!prefs.verify_pin("   "));
        assert!(!prefs.verify_pin(&"9".repeat(MAX_PIN_LEN + 1)));
        assert!(!ClientPrefs::default().verify_pin("1234"), "no PIN set");
    }

    #[test]
    fn set_pin_enforces_length_bounds() {
        let mut prefs = ClientPrefs::default();
        assert!(prefs.set_pin("123").is_err());
        assert!(prefs.set_pin(&"1".repeat(MAX_PIN_LEN + 1)).is_err());
        assert!(!prefs.has_pin(), "a rejected PIN must not be stored");
        assert!(prefs.set_pin("1234").is_ok());
        assert!(prefs.has_pin());
    }

    // Reaching `set_client_prefs` must not be enough to turn the lock off: a
    // page that can call the bridge would otherwise unlock the app by writing
    // `app_lock_enabled: false`.
    #[test]
    fn disabling_the_lock_requires_the_current_pin() {
        let mut prefs = prefs_with_pin("1234");

        let mut dto = dto_of(&prefs);
        dto.app_lock_enabled = false;
        assert_eq!(
            prefs.clone().apply_dto(dto).unwrap_err(),
            "Current PIN is incorrect"
        );
        assert!(prefs.app_lock_enabled && prefs.has_pin());

        let mut dto = dto_of(&prefs);
        dto.app_lock_enabled = false;
        dto.current_app_lock_pin = Some("9999".into());
        assert!(prefs.clone().apply_dto(dto).is_err());

        let mut dto = dto_of(&prefs);
        dto.app_lock_enabled = false;
        dto.current_app_lock_pin = Some("1234".into());
        prefs.apply_dto(dto).expect("correct PIN should disable");
        assert!(!prefs.app_lock_enabled);
        assert!(!prefs.has_pin(), "disabling clears the stored PIN");
    }

    #[test]
    fn changing_the_pin_requires_the_current_pin() {
        let mut prefs = prefs_with_pin("1234");

        let mut dto = dto_of(&prefs);
        dto.app_lock_pin = Some("5678".into());
        assert!(prefs.clone().apply_dto(dto).is_err());
        assert!(prefs.verify_pin("1234"), "the old PIN must survive");

        let mut dto = dto_of(&prefs);
        dto.app_lock_pin = Some("5678".into());
        dto.current_app_lock_pin = Some("1234".into());
        prefs.apply_dto(dto).expect("correct PIN should rotate");
        assert!(prefs.verify_pin("5678"));
        assert!(!prefs.verify_pin("1234"));
    }

    #[test]
    fn unrelated_prefs_change_without_the_pin() {
        let mut prefs = prefs_with_pin("1234");
        let mut dto = dto_of(&prefs);
        dto.wifi_only = false;
        dto.enable_logs = false;
        prefs.apply_dto(dto).expect("no PIN needed for other fields");
        assert!(!prefs.wifi_only);
        assert!(!prefs.enable_logs);
        assert!(prefs.app_lock_enabled && prefs.verify_pin("1234"));
    }

    #[test]
    fn the_lock_cannot_be_enabled_without_a_pin() {
        let mut prefs = ClientPrefs::default();
        let mut dto = dto_of(&prefs);
        dto.app_lock_enabled = true;
        assert!(prefs.apply_dto(dto).is_err());
        assert!(!prefs.app_lock_enabled);
    }

    #[test]
    fn cache_limit_is_clamped_to_the_supported_range() {
        let mut prefs = ClientPrefs::default();
        let mut dto = dto_of(&prefs);
        dto.cache_limit_bytes = u64::MAX;
        prefs.apply_dto(dto).unwrap();
        assert_eq!(prefs.cache_limit_bytes, MAX_CACHE_LIMIT_BYTES);

        let mut dto = dto_of(&prefs);
        dto.cache_limit_bytes = 1;
        prefs.apply_dto(dto).unwrap();
        assert_eq!(prefs.cache_limit_bytes, MIN_CACHE_LIMIT_BYTES);
    }

    // Prefs written by older builds carry a plaintext `app_lock_pin`. It has to
    // be rehashed on load and dropped from the file, not kept alongside.
    #[test]
    fn a_legacy_plaintext_pin_is_migrated_to_a_hash() {
        let stored = r#"{"wifi_only":true,"app_lock_enabled":true,"app_lock_pin":"1234"}"#;
        let mut prefs: ClientPrefs = serde_json::from_str(stored).unwrap();
        assert!(prefs.migrate_legacy_pin());
        assert!(prefs.has_pin());
        assert!(prefs.verify_pin("1234"));

        let json = serde_json::to_string(&prefs).unwrap();
        assert!(
            !json.contains("1234"),
            "the plaintext PIN must not be rewritten to disk: {json}"
        );
        assert!(!prefs.migrate_legacy_pin(), "migration runs once");
    }

    #[test]
    fn an_unusable_legacy_pin_turns_the_lock_off_instead_of_leaving_it_open() {
        let stored = r#"{"wifi_only":true,"app_lock_enabled":true,"app_lock_pin":"  "}"#;
        let mut prefs: ClientPrefs = serde_json::from_str(stored).unwrap();
        assert!(prefs.migrate_legacy_pin());
        assert!(!prefs.has_pin());
        assert!(
            !prefs.app_lock_enabled,
            "a lock with no verifiable PIN must not stay on"
        );
    }

    #[test]
    fn trusted_ipc_origins_reject_null_and_empty() {
        assert!(!SHELL_ORIGINS.contains(&""));
        assert!(!SHELL_ORIGINS.contains(&"null"));
        assert!(SHELL_ORIGINS.contains(&"tauri://localhost"));
    }

    #[test]
    fn origin_of_base_url_drops_path_and_keeps_port() {
        assert_eq!(
            origin_of_base_url("http://192.168.1.5:8080/files?tab=1").as_deref(),
            Some("http://192.168.1.5:8080")
        );
        assert_eq!(
            origin_of_base_url("https://sarca.example.com/").as_deref(),
            Some("https://sarca.example.com")
        );
        assert_eq!(origin_of_base_url("not a url"), None);
    }

    #[test]
    fn session_inject_is_pinned_to_the_server_origin() {
        let cfg = ServerConfig {
            base_url: "http://127.0.0.1:38827".into(),
            access_token: "a".into(),
            refresh_token: "r".into(),
            email: "e@example.com".into(),
            email_verified: true,
            ..Default::default()
        };
        let inject = SessionInject::from(&cfg);
        assert_eq!(inject.origin, "http://127.0.0.1:38827");
        let script = inject.eval_script();
        assert!(script.contains("location.origin !== want"));
    }

    #[test]
    fn shell_url_accepts_tauri_origins_only() {
        assert!(is_shell_url(
            &Url::parse("tauri://localhost/index.html").unwrap()
        ));
        assert!(is_shell_url(
            &Url::parse("https://tauri.localhost/").unwrap()
        ));
        assert!(is_shell_url(&Url::parse("http://localhost:1420/").unwrap()));
        // Sarca server on localhost must NOT be treated as the client shell.
        assert!(!is_shell_url(
            &Url::parse("http://127.0.0.1:8080/storages").unwrap()
        ));
        assert!(!is_shell_url(
            &Url::parse("https://localhost/sync.html").unwrap()
        ));
        assert!(!is_shell_url(&Url::parse("https://example.com/").unwrap()));
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
        let invoke = js.find("function __sarcaInvoke").expect("__sarcaInvoke");
        let body = &js[invoke..];
        let via_tauri_call = body.find("return viaTauri()").expect("prefer viaTauri");
        let fetch_call = body.find("__sarcaFetchInvoke").expect("fetch fallback");
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
        assert!(
            js.contains("__sarcaIsBridgeError"),
            "must distinguish bridge failures from command/API errors"
        );
        assert!(
            js.contains("__sarcaReadSession") && js.contains("update_session"),
            "must push webview tokens via update_session before Sync commands"
        );
        assert!(
            js.contains("__sarcaParseLs") && js.contains("JSON.parse"),
            "must JSON.parse localStorage (createLocalStore encoding) before Sync"
        );
        assert!(
            js.contains("__sarcaWatchSession"),
            "must watch localStorage so website login syncs tokens into native state"
        );
    }

    #[test]
    fn normalize_stored_token_strips_json_string_quotes() {
        // Website login: createLocalStore → localStorage value is `"jwt"` (with quotes).
        assert_eq!(
            normalize_stored_token("\"eyJhbGciOiJIUzI1NiJ9\""),
            "eyJhbGciOiJIUzI1NiJ9"
        );
        assert_eq!(normalize_stored_token("plain-token"), "plain-token");
        assert_eq!(normalize_stored_token("  "), "");
    }

    #[test]
    fn webview_session_from_json_encoded_local_storage() {
        let tokens = WebviewSessionTokens::from_local_storage_raw(
            Some("\"access-live\""),
            Some("\"refresh-live\""),
            Some("{\"email\":\"u@example.com\",\"email_verified\":true}"),
        )
        .expect("tokens");
        assert_eq!(tokens.access_token, "access-live");
        assert_eq!(tokens.refresh_token.as_deref(), Some("refresh-live"));
        assert_eq!(tokens.email.as_deref(), Some("u@example.com"));
        assert_eq!(tokens.email_verified, Some(true));
        assert!(tokens.has_access());
    }

    #[test]
    fn webview_session_empty_when_no_access_token() {
        assert!(WebviewSessionTokens::from_local_storage_raw(
            Some("\"\""),
            Some("\"refresh\""),
            None
        )
        .is_none());
        assert!(WebviewSessionTokens::from_local_storage_raw(None, None, None).is_none());
    }

    #[test]
    fn session_ready_only_when_native_or_webview_has_tokens() {
        assert!(!session_ready_for_sync(false, false));
        assert!(session_ready_for_sync(true, false));
        assert!(session_ready_for_sync(false, true));
        assert!(session_ready_for_sync(true, true));
    }

    #[test]
    fn merge_session_tokens_accepts_json_quoted_access() {
        let mut cfg = ServerConfig {
            base_url: "http://localhost:8001".into(),
            access_token: String::new(),
            ..Default::default()
        };
        merge_session_tokens(
            &mut cfg,
            "\"webview-token\"",
            Some("\"refresh\""),
            None,
            None,
        )
        .unwrap();
        assert_eq!(cfg.access_token, "webview-token");
        assert_eq!(cfg.refresh_token, "refresh");
        assert!(cfg.is_connected());
    }

    #[test]
    fn read_webview_session_js_parses_create_local_store_encoding() {
        assert!(READ_WEBVIEW_SESSION_JS.contains("JSON.parse"));
        assert!(READ_WEBVIEW_SESSION_JS.contains("access_token"));
        assert!(READ_WEBVIEW_SESSION_JS.contains("refresh_token"));
    }

    #[test]
    fn merge_session_tokens_updates_access_and_refresh() {
        let mut cfg = ServerConfig {
            base_url: "http://localhost:8001".into(),
            access_token: "old".into(),
            refresh_token: String::new(),
            email: String::new(),
            email_verified: false,
        };
        merge_session_tokens(
            &mut cfg,
            "new-access",
            Some("new-refresh"),
            Some("user@example.com"),
            Some(true),
        )
        .unwrap();
        assert_eq!(cfg.access_token, "new-access");
        assert_eq!(cfg.refresh_token, "new-refresh");
        assert_eq!(cfg.email, "user@example.com");
        assert!(cfg.email_verified);
        assert!(cfg.is_connected());
    }

    #[test]
    fn merge_session_tokens_rejects_empty_access() {
        let mut cfg = ServerConfig {
            base_url: "http://localhost:8001".into(),
            access_token: "old".into(),
            ..Default::default()
        };
        let err = merge_session_tokens(&mut cfg, "  ", None, None, None).unwrap_err();
        assert!(err.to_lowercase().contains("access token"));
        assert_eq!(cfg.access_token, "old");
    }

    #[test]
    fn record_url_history_dedupes_mru_and_caps_at_three() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sarca-url-history-{nanos}"));
        fs::create_dir_all(&dir).unwrap();

        assert!(load_url_history(&dir).is_empty());

        record_url_history(&dir, "https://a.example.com/");
        record_url_history(&dir, "https://b.example.com");
        let history = record_url_history(&dir, "https://c.example.com");
        assert_eq!(
            history,
            vec![
                "https://c.example.com",
                "https://b.example.com",
                "https://a.example.com",
            ]
        );

        // Re-visiting an existing entry moves it to the front instead of duplicating.
        let history = record_url_history(&dir, "https://a.example.com");
        assert_eq!(
            history,
            vec![
                "https://a.example.com",
                "https://c.example.com",
                "https://b.example.com",
            ]
        );

        // A fourth distinct URL evicts the oldest.
        let history = record_url_history(&dir, "https://d.example.com");
        assert_eq!(
            history,
            vec![
                "https://d.example.com",
                "https://a.example.com",
                "https://c.example.com",
            ]
        );
        assert_eq!(load_url_history(&dir), history);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_session_tokens_requires_base_url() {
        let mut cfg = ServerConfig::default();
        let err = merge_session_tokens(&mut cfg, "tok", None, None, None).unwrap_err();
        assert!(err.to_lowercase().contains("not connected"));
    }
}
