//! Remote-origin IPC for the server UI loaded inside the native WebView.
//!
//! Tauri does not inject `__TAURI_INTERNALS__` into arbitrary http(s) pages, so
//! Settings (Sync / Security / General) call `window.__sarcaInvoke`, which:
//! 1. Uses `__TAURI_INTERNALS__.invoke` when remote ACL allows,
//! 2. `fetch`es the `sarca-ipc` custom protocol, or
//! 3. falls back to a cancelled navigation to `https://sarca.ipc/...`.
//!
//! # Trust boundary
//!
//! Both fallbacks sit outside the Tauri capability ACL, so this module is the
//! only thing standing between a web page and the native command surface
//! (`add_binding`, `update_session`, `export_logs`, `disconnect`, …). Every
//! entry point must therefore establish the caller's origin and refuse anything
//! that is not the connected Sarca server or the bundled shell — an iframe, an
//! ad, a page the user was redirected to, a `data:` document. The responses
//! echo exactly that one origin: a wildcard `Access-Control-Allow-Origin` would
//! hand the whole bridge to any site the WebView ever loads.

use serde_json::{json, Value};
use tauri::{
    http::{header::CONTENT_TYPE, Request, Response, StatusCode},
    AppHandle, Manager, UriSchemeContext, UriSchemeResponder, Webview,
};

use crate::commands;
use crate::state::AppSyncState;

/// Largest IPC body we parse. `cache_put_preview` carries base64 image data, so
/// the limit has to clear a preview; everything above it is refused before it
/// reaches `serde_json`.
const MAX_IPC_BODY_BYTES: usize = 12 * 1024 * 1024;

/// Every command the remote Settings Sync / Security / General UI may call.
/// Keep in sync with `build.rs` AppManifest commands and `capabilities/default.json`.
pub const REMOTE_SETTINGS_COMMANDS: &[&str] = &[
    "platform_label",
    "device_label",
    "default_gallery_path",
    "is_on_wifi",
    "get_about",
    "get_session",
    "update_session",
    "get_client_prefs",
    "set_client_prefs",
    "verify_app_lock_pin",
    "export_logs",
    "list_storages",
    "list_bindings",
    "sync_statuses",
    "sync_transfer_queue",
    "set_app_foreground",
    "sync_now",
    "add_binding",
    "remove_binding",
    "set_binding_enabled",
    "update_binding_local_path",
    "update_binding_remote_root",
    "ensure_remote_folder",
    "pick_local_folder",
    "get_cache_size",
    "clear_local_cache",
    "cache_get_preview",
    "cache_put_preview",
    "open_sync_settings",
    "open_app",
    "disconnect",
];

/// Commands the bundled shell keeps to itself. `connect` picks the server the
/// whole trust model is anchored on and `get_url_history` lists every server the
/// user ever reached, so neither may be reachable from a remote page.
pub const SHELL_ONLY_COMMANDS: &[&str] = &["connect", "get_url_history"];

/// True when `cmd` is handled by [`dispatch`] (snake_case, exact match).
pub fn is_dispatched_command(cmd: &str) -> bool {
    REMOTE_SETTINGS_COMMANDS.contains(&cmd)
}

/// `allow-…` permission identifier for a dispatched command.
pub(crate) fn permission_for(cmd: &str) -> String {
    format!("allow-{}", cmd.replace('_', "-"))
}

/// URL pattern that matches exactly one origin, at any path.
pub(crate) fn origin_url_pattern(origin: &str) -> String {
    format!("{}/*", origin.trim_end_matches('/'))
}

/// Grant the connected Sarca server — and only it — the ACL for the Settings
/// commands.
///
/// `capabilities/default.json` used to carry `remote.urls: ["http://*:*/*",
/// "https://*:*/*"]`, which handed `__TAURI_INTERNALS__.invoke` to every http(s)
/// page the WebView ever loaded. The grant is now built from the origin the user
/// actually connected to, is limited to the Settings command set (no `connect`,
/// no `get_url_history`), and is applied to the `main` window only.
pub fn grant_remote_capability(app: &AppHandle, origin: &str) -> Result<(), String> {
    if origin.is_empty() {
        return Ok(());
    }
    let mut builder = tauri::ipc::CapabilityBuilder::new(format!("remote-server:{origin}"))
        .local(false)
        .remote(origin_url_pattern(origin))
        .window("main");
    for cmd in REMOTE_SETTINGS_COMMANDS {
        builder = builder.permission(permission_for(cmd));
    }
    app.add_capability(builder).map_err(|e| e.to_string())
}

/// Decide whether a `__TAURI_INTERNALS__.invoke` call may run, based on the URL
/// the calling webview is on *right now*.
///
/// Tauri 2.11 has `add_capability` and no matching `remove_capability`: the grant
/// [`grant_remote_capability`] hands to a server survives `disconnect` and every
/// later `connect`, for the whole process lifetime. So the ACL alone answers
/// "was this origin ever connected?", not "is it connected now" — and after the
/// user moves from server A to server B, a page still on A keeps a working
/// bridge to `get_session` (server B's tokens), `update_session`, `add_binding`,
/// `export_logs`. The `sarca-ipc` protocol handler and the cancelled-navigation
/// path already re-check the origin per call; this closes the third door, the
/// direct `invoke` the injected `__sarcaInvoke` prefers.
///
/// `url` is the webview's current URL — `None` means we could not read it, which
/// is refused rather than trusted.
pub fn authorize_invoke(app: &AppHandle, command: &str, url: Option<&tauri::Url>) -> bool {
    let Some(url) = url else {
        return false;
    };
    if crate::state::is_shell_url(url) {
        return true;
    }
    let Some(state) = app.try_state::<AppSyncState>() else {
        // Before `setup` managed the state nothing but the shell exists.
        return false;
    };
    if !state.is_trusted_ipc_url(url) {
        return false;
    }
    !SHELL_ONLY_COMMANDS.contains(&command)
}

fn arg_str(args: &Value, snake: &str, camel: &str) -> Option<String> {
    args.get(snake).or_else(|| args.get(camel)).and_then(|v| {
        if v.is_null() {
            None
        } else if let Some(s) = v.as_str() {
            Some(s.to_owned())
        } else {
            Some(v.to_string())
        }
    })
}

fn arg_value<'a>(args: &'a Value, snake: &str, camel: &str) -> Option<&'a Value> {
    args.get(snake).or_else(|| args.get(camel))
}

/// Build a response for `origin`, which the caller has already verified.
///
/// `Access-Control-Allow-Origin` names that single origin (never `*`) and
/// `Vary: Origin` keeps a cached reply from being replayed to another one.
fn ipc_response(
    status: StatusCode,
    body: Vec<u8>,
    content_type: &str,
    origin: &str,
) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header("Access-Control-Allow-Origin", origin)
        .header("Vary", "Origin")
        .header("Access-Control-Allow-Methods", "POST, OPTIONS")
        .header("Access-Control-Allow-Headers", "Content-Type")
        .header("Access-Control-Max-Age", "600")
        .header("Cache-Control", "no-store")
        .body(body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

/// Reply to a caller we refuse to serve. Carries no CORS headers at all, so the
/// requesting page cannot even read the status.
fn denied_response() -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header(CONTENT_TYPE, "application/json")
        .header("Cache-Control", "no-store")
        .body(br#"{"ok":false,"error":"origin not allowed"}"#.to_vec())
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

/// Resolve the origin a custom-protocol request came from, and return it only
/// when it is allowed to drive native commands.
///
/// Two independent conditions must hold:
/// * the `Origin` header names the connected server or the bundled shell, and
/// * the webview that issued the request is itself on such a page, so a
///   cross-origin iframe inside a trusted page cannot borrow the bridge.
fn authorize_request(
    app: &AppHandle,
    request: &Request<Vec<u8>>,
    webview_label: &str,
) -> Option<String> {
    let state = app.try_state::<AppSyncState>()?;

    let origin = request
        .headers()
        .get("Origin")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|o| !o.is_empty())?;

    if !state.is_trusted_ipc_origin(origin) {
        tracing::warn!(origin, "native IPC refused: untrusted origin");
        return None;
    }

    // `get_webview` is behind tauri's `unstable` feature; the app only ever
    // creates webview windows, whose webview label equals the window label.
    let page_ok = app
        .get_webview_window(webview_label)
        .and_then(|wv| wv.url().ok())
        .is_some_and(|url| state.is_trusted_ipc_url(&url));
    if !page_ok {
        tracing::warn!(origin, webview_label, "native IPC refused: untrusted page");
        return None;
    }

    Some(origin.to_owned())
}

/// Run a Settings / Sync command on behalf of the remote web UI.
pub async fn dispatch(app: AppHandle, cmd: &str, args: Value) -> Result<Value, String> {
    let state = app.state::<AppSyncState>();
    match cmd {
        "platform_label" => Ok(json!(commands::platform_label())),
        "device_label" => Ok(json!(commands::device_label(app.clone()))),
        "default_gallery_path" => Ok(json!(commands::default_gallery_path())),
        "is_on_wifi" => Ok(json!(commands::is_on_wifi())),
        "get_about" => Ok(serde_json::to_value(commands::get_about()).map_err(|e| e.to_string())?),
        "get_session" => {
            let dto = commands::get_session(state.clone()).await?;
            serde_json::to_value(dto).map_err(|e| e.to_string())
        }
        "update_session" => {
            let access_token = arg_str(&args, "access_token", "accessToken")
                .ok_or_else(|| "access_token required".to_string())?;
            let refresh_token = arg_str(&args, "refresh_token", "refreshToken");
            let email = arg_str(&args, "email", "email");
            let email_verified =
                arg_value(&args, "email_verified", "emailVerified").and_then(|v| v.as_bool());
            let dto = commands::update_session(
                state.clone(),
                access_token,
                refresh_token,
                email,
                email_verified,
            )
            .await?;
            serde_json::to_value(dto).map_err(|e| e.to_string())
        }
        "get_client_prefs" => {
            let prefs = commands::get_client_prefs(state.clone())?;
            serde_json::to_value(prefs).map_err(|e| e.to_string())
        }
        "set_client_prefs" => {
            let prefs_val = arg_value(&args, "prefs", "prefs")
                .cloned()
                .unwrap_or(args.clone());
            let prefs: crate::state::ClientPrefsDto =
                serde_json::from_value(prefs_val).map_err(|e| e.to_string())?;
            let saved = commands::set_client_prefs(state.clone(), prefs)?;
            serde_json::to_value(saved).map_err(|e| e.to_string())
        }
        "verify_app_lock_pin" => {
            let pin = arg_str(&args, "pin", "pin").unwrap_or_default();
            Ok(json!(commands::verify_app_lock_pin(state.clone(), pin)?))
        }
        "export_logs" => {
            let dto = commands::export_logs(app.clone(), state.clone()).await?;
            serde_json::to_value(dto).map_err(|e| e.to_string())
        }
        "list_storages" => {
            let list = commands::list_storages(app.clone(), state.clone()).await?;
            serde_json::to_value(list).map_err(|e| e.to_string())
        }
        "list_bindings" => {
            let list = commands::list_bindings(state.clone()).await?;
            serde_json::to_value(list).map_err(|e| e.to_string())
        }
        "sync_statuses" => {
            let list = commands::sync_statuses(state.clone()).await?;
            serde_json::to_value(list).map_err(|e| e.to_string())
        }
        "sync_transfer_queue" => {
            let snap = commands::sync_transfer_queue(state.clone()).await?;
            serde_json::to_value(snap).map_err(|e| e.to_string())
        }
        "set_app_foreground" => {
            let active = arg_value(&args, "active", "active")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| "active required".to_string())?;
            commands::set_app_foreground(state.clone(), active)?;
            Ok(json!(null))
        }
        "sync_now" => {
            let binding_id = arg_str(&args, "binding_id", "bindingId");
            commands::sync_now(app.clone(), state.clone(), binding_id).await?;
            Ok(json!(null))
        }
        "add_binding" => {
            let storage_id = arg_str(&args, "storage_id", "storageId")
                .ok_or_else(|| "storage_id required".to_string())?;
            let remote_root = arg_str(&args, "remote_root", "remoteRoot").unwrap_or_default();
            let local_path = arg_str(&args, "local_path", "localPath")
                .ok_or_else(|| "local_path required".to_string())?;
            let mode = arg_str(&args, "mode", "mode").unwrap_or_else(|| "folder_upload".into());
            let binding = commands::add_binding(
                app.clone(),
                state.clone(),
                storage_id,
                remote_root,
                local_path,
                mode,
            )
            .await?;
            serde_json::to_value(binding).map_err(|e| e.to_string())
        }
        "remove_binding" => {
            let id = arg_str(&args, "id", "id").ok_or_else(|| "id required".to_string())?;
            commands::remove_binding(state.clone(), id).await?;
            Ok(json!(null))
        }
        "set_binding_enabled" => {
            let id = arg_str(&args, "id", "id").ok_or_else(|| "id required".to_string())?;
            let enabled = arg_value(&args, "enabled", "enabled")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| "enabled required".to_string())?;
            commands::set_binding_enabled(state.clone(), id, enabled).await?;
            Ok(json!(null))
        }
        "update_binding_local_path" => {
            let id = arg_str(&args, "id", "id").ok_or_else(|| "id required".to_string())?;
            let local_path = arg_str(&args, "local_path", "localPath")
                .ok_or_else(|| "local_path required".to_string())?;
            let binding =
                commands::update_binding_local_path(app.clone(), state.clone(), id, local_path)
                    .await?;
            serde_json::to_value(binding).map_err(|e| e.to_string())
        }
        "update_binding_remote_root" => {
            let id = arg_str(&args, "id", "id").ok_or_else(|| "id required".to_string())?;
            let remote_root = arg_str(&args, "remote_root", "remoteRoot")
                .ok_or_else(|| "remote_root required".to_string())?;
            let binding =
                commands::update_binding_remote_root(state.clone(), id, remote_root).await?;
            serde_json::to_value(binding).map_err(|e| e.to_string())
        }
        "ensure_remote_folder" => {
            let storage_id = arg_str(&args, "storage_id", "storageId")
                .ok_or_else(|| "storage_id required".to_string())?;
            let parent = arg_str(&args, "parent", "parent").unwrap_or_default();
            let name = arg_str(&args, "name", "name").ok_or_else(|| "name required".to_string())?;
            let path = commands::ensure_remote_folder(
                app.clone(),
                state.clone(),
                storage_id,
                parent,
                name,
            )
            .await?;
            Ok(json!(path))
        }
        "pick_local_folder" => {
            let current = arg_str(&args, "current", "current");
            let path = commands::pick_local_folder(app.clone(), current).await?;
            Ok(json!(path))
        }
        "get_cache_size" => {
            let dto = commands::get_cache_size(state.clone())?;
            serde_json::to_value(dto).map_err(|e| e.to_string())
        }
        "clear_local_cache" => {
            let dto = commands::clear_local_cache(state.clone())?;
            serde_json::to_value(dto).map_err(|e| e.to_string())
        }
        "cache_get_preview" => {
            let scope =
                arg_str(&args, "scope", "scope").ok_or_else(|| "scope required".to_string())?;
            let path = arg_str(&args, "path", "path").unwrap_or_default();
            let b64 = commands::cache_get_preview(state.clone(), scope, path)?;
            Ok(json!(b64))
        }
        "cache_put_preview" => {
            let scope =
                arg_str(&args, "scope", "scope").ok_or_else(|| "scope required".to_string())?;
            let path = arg_str(&args, "path", "path").unwrap_or_default();
            let bytes_b64 = args
                .get("bytes_b64")
                .or_else(|| args.get("bytesB64"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .ok_or_else(|| "bytes_b64 required".to_string())?;
            commands::cache_put_preview(state.clone(), scope, path, bytes_b64)?;
            Ok(json!(null))
        }
        "open_sync_settings" => {
            commands::open_sync_settings(app.clone())?;
            Ok(json!(null))
        }
        "open_app" => {
            commands::open_app(app.clone(), state.clone()).await?;
            Ok(json!(null))
        }
        "disconnect" => {
            commands::disconnect(app.clone(), state.clone()).await?;
            Ok(json!(null))
        }
        other => {
            if is_dispatched_command(other) {
                // Listed but missing match arm — programming error.
                Err(format!("Unimplemented native command: {other}"))
            } else {
                Err(format!("Unknown native command: {other}"))
            }
        }
    }
}

fn resolve_script(id: &str, ok: bool, value: &Value) -> String {
    let payload = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    format!(
        "(function(){{ try {{ if (typeof window.__sarcaIpcResolve === 'function') window.__sarcaIpcResolve({id}, {ok}, {payload}); }} catch (e) {{}} }})();",
        id = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into()),
        ok = if ok { "true" } else { "false" },
        payload = payload
    )
}

/// True when this navigation is our remote IPC channel (must cancel load).
pub fn is_ipc_url(url: &tauri::Url) -> bool {
    if url.scheme() == "sarca-ipc" {
        return true;
    }
    matches!(url.scheme(), "http" | "https")
        && url.host_str() == Some("sarca.ipc")
        && url.path().starts_with("/__invoke__")
}

/// Parse IPC request from a cancelled navigation URL and run it.
///
/// Returns `true` for every IPC URL, allowed or not, so the caller always
/// cancels the navigation — a refused request must not be allowed to load.
pub fn handle_navigation(webview: &Webview, url: &tauri::Url) -> bool {
    if !is_ipc_url(url) {
        return false;
    }

    // `on_navigation` fires before the new document commits, so the webview is
    // still on the page that issued the call: exactly the origin to authorise.
    let trusted = webview
        .app_handle()
        .try_state::<AppSyncState>()
        .is_some_and(|state| {
            webview
                .url()
                .ok()
                .is_some_and(|current| state.is_trusted_ipc_url(&current))
        });
    if !trusted {
        tracing::warn!("native IPC refused: navigation from an untrusted page");
        return true;
    }

    let payload = url
        .query_pairs()
        .find(|(k, _)| k == "p")
        .map(|(_, v)| v.into_owned());

    let Some(raw) = payload else {
        return true;
    };

    if raw.len() > MAX_IPC_BODY_BYTES {
        return true;
    }

    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            let _ = webview.eval(resolve_script(
                "unknown",
                false,
                &json!(format!("Invalid IPC payload: {e}")),
            ));
            return true;
        }
    };

    let id = parsed
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned();
    let cmd = parsed
        .get("cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let args = parsed.get("args").cloned().unwrap_or_else(|| json!({}));

    let app = webview.app_handle().clone();
    let wv = webview.clone();
    tauri::async_runtime::spawn(async move {
        let result = dispatch(app, &cmd, args).await;
        let script = match result {
            Ok(val) => resolve_script(&id, true, &val),
            Err(err) => resolve_script(&id, false, &json!(err)),
        };
        let _ = wv.eval(&script);
    });

    true
}

/// Custom protocol handler: `fetch('sarca-ipc://localhost/invoke')` from remote UI.
/// On Windows/Android the webview rewrites this to `http://sarca-ipc.localhost/invoke`.
pub fn handle_protocol(
    ctx: UriSchemeContext<'_, tauri::Wry>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let Some(origin) = authorize_request(&app, &request, ctx.webview_label()) else {
        responder.respond(denied_response());
        return;
    };

    if request.method() == tauri::http::Method::OPTIONS {
        responder.respond(ipc_response(
            StatusCode::NO_CONTENT,
            Vec::new(),
            "text/plain",
            &origin,
        ));
        return;
    }

    // A GET carries its payload in the URL, where it lands in history and logs,
    // and is reachable from a plain `<img>`/`<script>` tag that no CORS
    // preflight guards. Commands are state-changing: require POST.
    if request.method() != tauri::http::Method::POST {
        responder.respond(ipc_response(
            StatusCode::METHOD_NOT_ALLOWED,
            br#"{"ok":false,"error":"POST required"}"#.to_vec(),
            "application/json",
            &origin,
        ));
        return;
    }

    if request.body().len() > MAX_IPC_BODY_BYTES {
        responder.respond(ipc_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            br#"{"ok":false,"error":"IPC body too large"}"#.to_vec(),
            "application/json",
            &origin,
        ));
        return;
    }

    tauri::async_runtime::spawn(async move {
        let body = request.body();
        if body.is_empty() {
            responder.respond(ipc_response(
                StatusCode::BAD_REQUEST,
                br#"{"ok":false,"error":"empty IPC body"}"#.to_vec(),
                "application/json",
                &origin,
            ));
            return;
        }

        let parsed: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => {
                responder.respond(ipc_response(
                    StatusCode::BAD_REQUEST,
                    format!(r#"{{"ok":false,"error":{}}}"#, json!(e.to_string())).into_bytes(),
                    "application/json",
                    &origin,
                ));
                return;
            }
        };

        let cmd = parsed
            .get("cmd")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let args = parsed.get("args").cloned().unwrap_or_else(|| json!({}));

        let result = dispatch(app, &cmd, args).await;
        let (status, body) = match result {
            Ok(val) => (
                StatusCode::OK,
                json!({ "ok": true, "value": val }).to_string().into_bytes(),
            ),
            Err(err) => (
                StatusCode::OK,
                json!({ "ok": false, "error": err })
                    .to_string()
                    .into_bytes(),
            ),
        };
        responder.respond(ipc_response(status, body, "application/json", &origin));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_command_names_are_snake_case_exact() {
        for cmd in REMOTE_SETTINGS_COMMANDS {
            assert!(
                cmd.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "command must be snake_case: {cmd}"
            );
            assert!(
                is_dispatched_command(cmd),
                "is_dispatched_command missed {cmd}"
            );
        }
        assert!(!is_dispatched_command("defaultGalleryPath"));
        assert!(!is_dispatched_command("DefaultGalleryPath"));
        assert!(!is_dispatched_command("pickLocalFolder"));
    }

    #[test]
    fn sync_security_commands_are_dispatched() {
        for cmd in [
            "default_gallery_path",
            "pick_local_folder",
            "set_client_prefs",
            "get_client_prefs",
            "export_logs",
            "add_binding",
            "remove_binding",
            "ensure_remote_folder",
            "update_session",
            "list_bindings",
            "sync_now",
            "sync_statuses",
            "sync_transfer_queue",
            "set_app_foreground",
            "is_on_wifi",
            "get_about",
            "get_cache_size",
            "clear_local_cache",
            "cache_get_preview",
            "cache_put_preview",
            "platform_label",
            "device_label",
            "set_binding_enabled",
            "update_binding_local_path",
            "update_binding_remote_root",
        ] {
            assert!(
                is_dispatched_command(cmd),
                "Settings Sync/Security command missing from dispatch: {cmd}"
            );
        }
    }

    #[test]
    fn soft_disable_commands_are_dispatched() {
        assert!(is_dispatched_command("set_binding_enabled"));
        assert!(is_dispatched_command("update_binding_local_path"));
        assert!(is_dispatched_command("update_binding_remote_root"));
    }

    #[test]
    fn ipc_url_detects_navigation_and_scheme() {
        assert!(is_ipc_url(
            &tauri::Url::parse("https://sarca.ipc/__invoke__?p=%7B%7D").unwrap()
        ));
        assert!(is_ipc_url(
            &tauri::Url::parse("http://sarca.ipc/__invoke__?p=1").unwrap()
        ));
        assert!(is_ipc_url(
            &tauri::Url::parse("sarca-ipc://localhost/__invoke__?p=1").unwrap()
        ));
        assert!(is_ipc_url(
            &tauri::Url::parse("sarca-ipc://localhost/invoke").unwrap()
        ));
        assert!(!is_ipc_url(
            &tauri::Url::parse("https://example.com/__invoke__").unwrap()
        ));
    }
}
