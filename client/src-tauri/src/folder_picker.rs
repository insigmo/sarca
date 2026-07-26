//! Android SAF folder picker plugin (ACTION_OPEN_DOCUMENT_TREE).

use tauri::{
    plugin::{Builder as PluginBuilder, TauriPlugin},
    Runtime,
};

/// Register the Android folder-picker plugin (no-op setup on other targets).
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    PluginBuilder::<R, ()>::new("folder-picker")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api
                    .register_android_plugin("app.sarca.client.folderpicker", "FolderPickerPlugin")?;
                app.manage(AndroidFolderPicker { handle });
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
struct AndroidFolderPicker<R: Runtime> {
    handle: tauri::plugin::PluginHandle<R>,
}

/// Open the Android document-tree picker and return a filesystem path when resolvable.
#[cfg(target_os = "android")]
pub async fn pick_folder_android<R: Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<Option<String>, String> {
    use serde::Deserialize;
    use tauri::Manager;
    use tokio::sync::oneshot;

    #[derive(Debug, Deserialize)]
    struct FolderPickResponse {
        path: Option<String>,
        #[allow(dead_code)]
        uri: Option<String>,
    }

    let state = app
        .try_state::<AndroidFolderPicker<R>>()
        .ok_or_else(|| "Android folder picker plugin not registered".to_string())?;
    let handle = state.handle.clone();

    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = handle.run_mobile_plugin::<FolderPickResponse>("pickFolder", ());
        let _ = tx.send(result);
    });

    match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
        Ok(Ok(Ok(resp))) => {
            if let Some(path) = resp.path.filter(|p| !p.is_empty()) {
                return Ok(Some(path));
            }
            // Tree URI without a resolvable FS path — UI may offer typed fallback.
            Err("FOLDER_PICKER_USE_PROMPT".into())
        }
        Ok(Ok(Err(e))) => {
            let msg = e.to_string();
            let lower = msg.to_lowercase();
            if lower.contains("cancel") {
                Ok(None)
            } else if lower.contains("not registered")
                || lower.contains("classnotfound")
                || lower.contains("failed to find class")
            {
                // Do NOT map to FOLDER_PICKER_USE_PROMPT — that hides a packaging
                // bug behind window.prompt. Rebuild with patch-android-http.sh.
                Err(format!(
                    "Android SAF folder picker unavailable ({msg}). Rebuild the APK after running scripts/patch-android-http.sh."
                ))
            } else {
                Err(msg)
            }
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Folder picker timed out".into()),
    }
}
