//! Android startup helpers: runtime permissions, device model, the previous
//! process's exit reason, saving a text file.
//! Also owns the on-disk device-label cache used by Sync Camera roots.

use std::{
    fs,
    path::{Path, PathBuf},
};

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
    // User may take a while on the system permission dialog; do not race the grant.
    match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
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
    let resp: DeviceModelResponse = state.handle.run_mobile_plugin("deviceModel", ()).ok()?;
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

/// Writes how the previous process ended into the client log, once per start.
///
/// The process can end with nothing in the log: a crash, the system reclaiming
/// memory, or the app calling `exit(0)` itself — which is what Tauri does when
/// its last activity is destroyed, and what stopped auto-upload every time the
/// app was swiped out of Recents (see `MainActivity.kt`). Telling those apart
/// took a phone on a cable (`dumpsys activity exit-info`); this puts the same
/// record into the log the user exports.
#[cfg(target_os = "android")]
pub fn log_last_exit<R: Runtime>(app: &tauri::AppHandle<R>, data_dir: PathBuf) {
    use tauri::Manager;

    let Some(state) = app.try_state::<AndroidStartup<R>>() else {
        return;
    };
    let handle = state.handle.clone();
    // `run_mobile_plugin` waits on the main looper; keep it off the caller.
    std::thread::spawn(move || {
        if let Ok(exit) = handle.run_mobile_plugin::<LastExit>("lastExit", ()) {
            if let Some(line) = exit.summary() {
                crate::client_log::write_line(&data_dir, &line);
            }
        }
    });
}

/// `StartupPlugin.lastExit`'s answer: Android's `ApplicationExitInfo` for the
/// previous process of this app. Every field is absent when there is no record.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastExit {
    reason: Option<i32>,
    status: Option<i32>,
    importance: Option<i32>,
    timestamp_ms: Option<i64>,
    rss_kb: Option<i64>,
    description: Option<String>,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
impl LastExit {
    /// One log line, or `None` when Android had nothing on record.
    pub fn summary(&self) -> Option<String> {
        let reason = self.reason?;
        let mut line = format!(
            "previous process exit: reason={} ({reason}) status={}",
            exit_reason_name(reason),
            self.status.unwrap_or(0)
        );
        if let Some(importance) = self.importance {
            // 100 = foreground, 125 = foreground service, 400+ = cached.
            line.push_str(&format!(" importance={importance}"));
        }
        if let Some(kb) = self.rss_kb {
            line.push_str(&format!(" rss={}MB", kb / 1024));
        }
        if let Some(ms) = self.timestamp_ms {
            line.push_str(&format!(" at_unix={}", ms / 1000));
        }
        if let Some(d) = self.description.as_deref().filter(|d| !d.is_empty()) {
            line.push_str(&format!(" description={d:?}"));
        }
        Some(line)
    }
}

/// `ApplicationExitInfo.REASON_*` by value.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn exit_reason_name(reason: i32) -> &'static str {
    match reason {
        1 => "EXIT_SELF",
        2 => "SIGNALED",
        3 => "LOW_MEMORY",
        4 => "CRASH",
        5 => "CRASH_NATIVE",
        6 => "ANR",
        7 => "INITIALIZATION_FAILURE",
        8 => "PERMISSION_CHANGE",
        9 => "EXCESSIVE_RESOURCE_USAGE",
        10 => "USER_REQUESTED",
        11 => "USER_STOPPED",
        12 => "DEPENDENCY_DIED",
        13 => "OTHER",
        14 => "FREEZER",
        15 => "PACKAGE_STATE_CHANGE",
        16 => "PACKAGE_UPDATED",
        _ => "UNKNOWN",
    }
}

/// Let the user pick where to save `text` (Android's "Save as" screen) and
/// write it there. `Ok(Some(name))` is the saved file's display name,
/// `Ok(None)` means the user backed out.
#[cfg(target_os = "android")]
pub async fn save_text_file<R: Runtime>(
    app: &tauri::AppHandle<R>,
    text: &str,
    file_name: &str,
) -> Result<Option<String>, String> {
    use serde::Deserialize;
    use serde_json::json;
    use tauri::Manager;
    use tokio::sync::oneshot;

    #[derive(Deserialize)]
    struct Saved {
        saved: bool,
        name: Option<String>,
    }

    let state = app
        .try_state::<AndroidStartup<R>>()
        .ok_or_else(|| "Android startup plugin not registered".to_string())?;
    let handle = state.handle.clone();
    let payload = json!({ "text": text, "fileName": file_name });
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = handle.run_mobile_plugin::<Saved>("saveTextFile", payload);
        let _ = tx.send(result);
    });
    // The user is browsing folders on the save screen; give them time.
    match tokio::time::timeout(std::time::Duration::from_secs(15 * 60), rx).await {
        Ok(Ok(Ok(r))) if r.saved => Ok(Some(r.name.unwrap_or_default())),
        Ok(Ok(Ok(_))) => Ok(None),
        Ok(Ok(Err(e))) => Err(e.to_string()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("saving the log timed out".into()),
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

/// True when `s` is safe to show as Camera/<label> (not empty / localhost).
pub fn is_usable_device_label(s: &str) -> bool {
    let cleaned = sanitize_device_label(s);
    !cleaned.is_empty() && !is_useless_hostname(&cleaned)
}

fn device_label_cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("device_label.txt")
}

/// Read a previously persisted device label from the app data dir.
pub fn read_device_label_cache(data_dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(device_label_cache_path(data_dir)).ok()?;
    let cleaned = sanitize_device_label(&raw);
    if is_usable_device_label(&cleaned) {
        Some(cleaned)
    } else {
        None
    }
}

/// Persist a usable device label for immediate Sync UI / binding use later.
pub fn write_device_label_cache(data_dir: &Path, label: &str) -> Result<(), String> {
    let cleaned = sanitize_device_label(label);
    if !is_usable_device_label(&cleaned) {
        return Err("device label is empty or useless".into());
    }
    if let Some(parent) = device_label_cache_path(data_dir).parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(device_label_cache_path(data_dir), &cleaned).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn last_exit_summary_names_the_reason() {
        let exit: LastExit = serde_json::from_str(
            r#"{"reason":1,"status":0,"importance":125,"timestampMs":1793111685302,"rssKb":1153433,"description":""}"#,
        )
        .unwrap();
        assert_eq!(
            exit.summary().as_deref(),
            Some(
                "previous process exit: reason=EXIT_SELF (1) status=0 importance=125 \
                 rss=1126MB at_unix=1793111685"
            )
        );
    }

    #[test]
    fn last_exit_summary_is_none_without_a_record() {
        let exit: LastExit = serde_json::from_str("{}").unwrap();
        assert_eq!(exit.summary(), None);
    }

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

    #[test]
    fn usable_device_label_rejects_empty_and_localhost() {
        assert!(!is_usable_device_label(""));
        assert!(!is_usable_device_label("   "));
        assert!(!is_usable_device_label("localhost"));
        assert!(!is_usable_device_label("127.0.0.1"));
        assert!(is_usable_device_label("Pixel 8"));
        assert!(is_usable_device_label("Google Pixel 8"));
    }

    #[test]
    fn device_label_cache_round_trips_and_skips_useless() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sarca-device-label-{nanos}"));
        fs::create_dir_all(&dir).unwrap();

        assert_eq!(read_device_label_cache(&dir), None);
        assert!(write_device_label_cache(&dir, "localhost").is_err());
        assert_eq!(read_device_label_cache(&dir), None);

        write_device_label_cache(&dir, "  Pixel/8\\Pro  ").unwrap();
        assert_eq!(
            read_device_label_cache(&dir).as_deref(),
            Some("Pixel 8 Pro")
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
