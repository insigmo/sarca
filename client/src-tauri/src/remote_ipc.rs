//! Remote-origin IPC for the server UI loaded inside the native WebView.
//!
//! Tauri does not inject `__TAURI_INTERNALS__` into arbitrary http(s) pages, so
//! Settings (Sync / Security / General) call `window.__sarcaInvoke`, which:
//! 1. Uses `__TAURI_INTERNALS__.invoke` when remote ACL allows,
//! 2. `fetch`es the `sarca-ipc` custom protocol, or
//! 3. falls back to a cancelled navigation to `https://sarca.ipc/...`.

use serde_json::{json, Value};
use tauri::{
    http::{header::CONTENT_TYPE, Request, Response, StatusCode},
    AppHandle, Manager, UriSchemeContext, UriSchemeResponder, Webview,
};

use crate::commands;
use crate::state::AppSyncState;

/// Every command the remote Settings Sync / Security / General UI may call.
/// Keep in sync with `build.rs` AppManifest commands and `capabilities/default.json`.
pub const REMOTE_SETTINGS_COMMANDS: &[&str] = &[
    "platform_label",
    "default_gallery_path",
    "is_on_wifi",
    "get_about",
    "get_session",
    "update_session",
    "get_client_prefs",
    "set_client_prefs",
    "list_storages",
    "list_bindings",
    "sync_statuses",
    "sync_now",
    "add_binding",
    "remove_binding",
    "ensure_remote_folder",
    "pick_local_folder",
    "get_cache_size",
    "clear_local_cache",
    "open_sync_settings",
    "open_app",
    "disconnect",
];

/// True when `cmd` is handled by [`dispatch`] (snake_case, exact match).
pub fn is_dispatched_command(cmd: &str) -> bool {
    REMOTE_SETTINGS_COMMANDS.contains(&cmd)
}

fn arg_str(args: &Value, snake: &str, camel: &str) -> Option<String> {
    args.get(snake)
        .or_else(|| args.get(camel))
        .and_then(|v| {
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

fn cors_response(status: StatusCode, body: Vec<u8>, content_type: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header(
            "Access-Control-Allow-Headers",
            "Content-Type, Authorization, X-Requested-With",
        )
        .header("Access-Control-Max-Age", "86400")
        .body(body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

/// Run a Settings / Sync command on behalf of the remote web UI.
pub async fn dispatch(app: AppHandle, cmd: &str, args: Value) -> Result<Value, String> {
    let state = app.state::<AppSyncState>();
    match cmd {
        "platform_label" => Ok(json!(commands::platform_label())),
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
            let email_verified = arg_value(&args, "email_verified", "emailVerified")
                .and_then(|v| v.as_bool());
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
            let prefs: crate::state::ClientPrefs =
                serde_json::from_value(prefs_val).map_err(|e| e.to_string())?;
            let saved = commands::set_client_prefs(state.clone(), prefs)?;
            serde_json::to_value(saved).map_err(|e| e.to_string())
        }
        "list_storages" => {
            let list = commands::list_storages(app.clone(), state.clone()).await?;
            serde_json::to_value(list).map_err(|e| e.to_string())
        }
        "list_bindings" => {
            let list = commands::list_bindings(state.clone())?;
            serde_json::to_value(list).map_err(|e| e.to_string())
        }
        "sync_statuses" => {
            let list = commands::sync_statuses(state.clone()).await?;
            serde_json::to_value(list).map_err(|e| e.to_string())
        }
        "sync_now" => {
            commands::sync_now(app.clone(), state.clone()).await?;
            Ok(json!(null))
        }
        "add_binding" => {
            let storage_id = arg_str(&args, "storage_id", "storageId")
                .ok_or_else(|| "storage_id required".to_string())?;
            let remote_root = arg_str(&args, "remote_root", "remoteRoot").unwrap_or_default();
            let local_path = arg_str(&args, "local_path", "localPath")
                .ok_or_else(|| "local_path required".to_string())?;
            let mode = arg_str(&args, "mode", "mode").unwrap_or_else(|| "sync".into());
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
            commands::remove_binding(state.clone(), id)?;
            Ok(json!(null))
        }
        "ensure_remote_folder" => {
            let storage_id = arg_str(&args, "storage_id", "storageId")
                .ok_or_else(|| "storage_id required".to_string())?;
            let parent = arg_str(&args, "parent", "parent").unwrap_or_default();
            let name =
                arg_str(&args, "name", "name").ok_or_else(|| "name required".to_string())?;
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
            let path = commands::pick_local_folder(app.clone()).await?;
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
            "add_binding",
            "remove_binding",
            "ensure_remote_folder",
            "update_session",
            "list_bindings",
            "sync_now",
            "sync_statuses",
            "is_on_wifi",
            "get_about",
            "get_cache_size",
            "clear_local_cache",
            "platform_label",
        ] {
            assert!(
                is_dispatched_command(cmd),
                "Settings Sync/Security command missing from dispatch: {cmd}"
            );
        }
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
pub fn handle_navigation(webview: &Webview, url: &tauri::Url) -> bool {
    if !is_ipc_url(url) {
        return false;
    }

    let payload = url
        .query_pairs()
        .find(|(k, _)| k == "p")
        .map(|(_, v)| v.into_owned());

    let Some(raw) = payload else {
        return true;
    };

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
    if request.method() == tauri::http::Method::OPTIONS {
        responder.respond(cors_response(StatusCode::NO_CONTENT, Vec::new(), "text/plain"));
        return;
    }

    let app = ctx.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let body = request.body();
        let parsed: Value = if body.is_empty() {
            // Allow GET ?p=<json> as well.
            let q = request.uri().query().unwrap_or("");
            let mut p = None;
            for pair in q.split('&') {
                if let Some(rest) = pair.strip_prefix("p=") {
                    p = Some(
                        urlencoding_decode(rest).unwrap_or_else(|| rest.to_owned()),
                    );
                    break;
                }
            }
            match p.as_deref().map(serde_json::from_str) {
                Some(Ok(v)) => v,
                Some(Err(e)) => {
                    responder.respond(cors_response(
                        StatusCode::BAD_REQUEST,
                        format!(r#"{{"ok":false,"error":{}}}"#, json!(e.to_string())).into_bytes(),
                        "application/json",
                    ));
                    return;
                }
                None => {
                    responder.respond(cors_response(
                        StatusCode::BAD_REQUEST,
                        br#"{"ok":false,"error":"empty IPC body"}"#.to_vec(),
                        "application/json",
                    ));
                    return;
                }
            }
        } else {
            match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    responder.respond(cors_response(
                        StatusCode::BAD_REQUEST,
                        format!(r#"{{"ok":false,"error":{}}}"#, json!(e.to_string())).into_bytes(),
                        "application/json",
                    ));
                    return;
                }
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
                json!({ "ok": false, "error": err }).to_string().into_bytes(),
            ),
        };
        responder.respond(cors_response(status, body, "application/json"));
    });
}

fn urlencoding_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = (bytes[i + 1] as char).to_digit(16)?;
            let l = (bytes[i + 2] as char).to_digit(16)?;
            out.push((h * 16 + l) as u8);
            i += 3;
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
