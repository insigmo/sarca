//! Android startup helpers: runtime permissions, device model, share text.

use tauri::{
    plugin::{Builder as PluginBuilder, TauriPlugin},
    Runtime,
};

/// Register the Android startup plugin (no-op setup on other targets).
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    PluginBuilder::<R, ()>::new("sarca-startup")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle =
                    api.register_android_plugin("app.sarca.client.startup", "StartupPlugin")?;
                app.manage(AndroidStartup { handle });
            }
            #[cfg(not(target_os = "android"))]
            {
                let _ = (app, api);
            }
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
struct AndroidStartup<R: Runtime> {
    handle: tauri::plugin::PluginHandle<R>,
}

/// Prompt for media read + battery-optimization exemption (best-effort).
#[cfg(target_os = "android")]
pub async fn ensure_runtime_access<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    use tauri::Manager;
    use tokio::sync::oneshot;

    let state = app
        .try_state::<AndroidStartup<R>>()
        .ok_or_else(|| "Android startup plugin not registered".to_string())?;
    let handle = state.handle.clone();
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = handle.run_mobile_plugin::<serde_json::Value>("ensureRuntimeAccess", ());
        let _ = tx.send(result);
    });
    match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
        Ok(Ok(Ok(_))) => Ok(()),
        Ok(Ok(Err(e))) => Err(e.to_string()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("ensureRuntimeAccess timed out".into()),
    }
}

/// User-friendly device label from `Build.MODEL` / manufacturer.
#[cfg(target_os = "android")]
pub fn device_model_label<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<String> {
    use serde::Deserialize;
    use tauri::Manager;

    #[derive(Debug, Deserialize)]
    struct DeviceModelResponse {
        label: Option<String>,
        model: Option<String>,
    }

    let state = app.try_state::<AndroidStartup<R>>()?;
    let resp: DeviceModelResponse = state
        .handle
        .run_mobile_plugin("deviceModel", ())
        .ok()?;
    let raw = resp
        .label
        .filter(|s| !s.trim().is_empty())
        .or_else(|| resp.model.filter(|s| !s.trim().is_empty()))?;
    let cleaned = sanitize_device_label(&raw);
    if cleaned.is_empty() || is_useless_hostname(&cleaned) {
        None
    } else {
        Some(cleaned)
    }
}

/// Share plain text via Android ACTION_SEND chooser.
#[cfg(target_os = "android")]
pub async fn share_text<R: Runtime>(
    app: &tauri::AppHandle<R>,
    text: &str,
    subject: &str,
) -> Result<(), String> {
    use serde_json::json;
    use tauri::Manager;
    use tokio::sync::oneshot;

    let state = app
        .try_state::<AndroidStartup<R>>()
        .ok_or_else(|| "Android startup plugin not registered".to_string())?;
    let handle = state.handle.clone();
    let payload = json!({ "text": text, "subject": subject });
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = handle.run_mobile_plugin::<serde_json::Value>("shareText", payload);
        let _ = tx.send(result);
    });
    match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
        Ok(Ok(Ok(_))) => Ok(()),
        Ok(Ok(Err(e))) => Err(e.to_string()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("shareText timed out".into()),
    }
}

pub fn sanitize_device_label(raw: &str) -> String {
    raw.replace(['/', '\\'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

pub fn is_useless_hostname(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    lower.is_empty()
        || lower == "localhost"
        || lower == "localhost.localdomain"
        || lower == "127.0.0.1"
        || lower == "(none)"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_localhost_hostnames() {
        assert!(is_useless_hostname("localhost"));
        assert!(is_useless_hostname("LocalHost"));
        assert!(is_useless_hostname("127.0.0.1"));
        assert!(!is_useless_hostname("Pixel 8"));
    }

    #[test]
    fn sanitize_strips_path_chars() {
        assert_eq!(sanitize_device_label("Foo/Bar\\Baz"), "Foo Bar Baz");
    }
}
