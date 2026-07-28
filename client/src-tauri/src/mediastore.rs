//! Android MediaStore bridge: DCIM discovery + upload materialization.
//!
//! Desktop / non-Android platforms never construct [`AndroidDcimMediaSource`]
//! (see `state.rs`); this module still compiles there so `cargo test --lib`
//! covers the platform-independent path-join helper.

use std::path::Path;

use async_trait::async_trait;
use sarca_sync::{collect_fs_candidates, strip_dcim_prefix, Binding, LocalCandidate, LocalMediaSource};
use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder as PluginBuilder, TauriPlugin},
    AppHandle, Runtime,
};

/// Register the Android MediaStore plugin (no-op setup on other targets).
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    PluginBuilder::<R, ()>::new("sarca-mediastore")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api
                    .register_android_plugin("app.sarca.client.mediastore", "MediaStorePlugin")?;
                app.manage(AndroidMediaStore { handle });
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
struct AndroidMediaStore<R: Runtime> {
    handle: tauri::plugin::PluginHandle<R>,
}

/// One DCIM image/video as reported by `listDcimMedia`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaStoreItem {
    pub uri: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "relativePath")]
    pub relative_path: String,
    pub size: i64,
    #[serde(rename = "dateModifiedMs")]
    pub date_modified_ms: i64,
}

/// List DCIM images/videos visible to MediaStore.
#[cfg(target_os = "android")]
pub async fn list_dcim_media<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<MediaStoreItem>, String> {
    use tauri::Manager;
    use tokio::sync::oneshot;

    #[derive(Debug, Deserialize)]
    struct ListDcimResponse {
        items: Vec<MediaStoreItem>,
    }

    let state = app
        .try_state::<AndroidMediaStore<R>>()
        .ok_or_else(|| "Android MediaStore plugin not registered".to_string())?;
    let handle = state.handle.clone();
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = handle.run_mobile_plugin::<ListDcimResponse>("listDcimMedia", ());
        let _ = tx.send(result);
    });
    match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
        Ok(Ok(Ok(resp))) => Ok(resp.items),
        Ok(Ok(Err(e))) => Err(e.to_string()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("listDcimMedia timed out".into()),
    }
}

/// Materialize a MediaStore `content://` URI into a real filesystem path for
/// upload. `ephemeral` is `true` when the path is a cache copy the caller may
/// delete after upload (no stable on-disk original was resolvable).
#[cfg(target_os = "android")]
pub async fn materialize_for_upload<R: Runtime>(
    app: &AppHandle<R>,
    uri: &str,
) -> Result<(std::path::PathBuf, bool), String> {
    use tauri::Manager;
    use tokio::sync::oneshot;

    #[derive(Debug, Deserialize)]
    struct MaterializeResponse {
        path: String,
        ephemeral: bool,
    }

    let state = app
        .try_state::<AndroidMediaStore<R>>()
        .ok_or_else(|| "Android MediaStore plugin not registered".to_string())?;
    let handle = state.handle.clone();
    let uri = uri.to_string();
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = handle.run_mobile_plugin::<MaterializeResponse>(
            "materializeForUpload",
            serde_json::json!({ "uri": uri }),
        );
        let _ = tx.send(result);
    });
    match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
        Ok(Ok(Ok(resp))) => Ok((std::path::PathBuf::from(resp.path), resp.ephemeral)),
        Ok(Ok(Err(e))) => Err(e.to_string()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("materializeForUpload timed out".into()),
    }
}

/// Join a MediaStore `relativePath` + `displayName`, then strip the leading
/// `DCIM/` so Camera roots line up with sarca-sync's remote layout
/// (`DCIM/Camera/IMG_1.jpg` → `Camera/IMG_1.jpg`).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn media_item_relative_path(relative_path: &str, display_name: &str) -> String {
    let joined = format!(
        "{}/{}",
        relative_path.trim_matches('/'),
        display_name.trim_matches('/')
    );
    strip_dcim_prefix(&joined)
}

/// `LocalMediaSource` backed by Android MediaStore for `AutoUpload` bindings;
/// falls back to a plain filesystem walk for `FolderUpload`/`Sync` bindings
/// (MediaStore only indexes DCIM, not arbitrary picked folders).
///
/// Only ever constructed on Android (see `state.rs`); it still compiles on
/// other targets so `AppSyncState::new` type-checks for every host platform.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub struct AndroidDcimMediaSource<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> AndroidDcimMediaSource<R> {
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }

    #[cfg(target_os = "android")]
    async fn list_dcim_via_mediastore(&self) -> anyhow::Result<Vec<LocalCandidate>> {
        use anyhow::anyhow;

        let items = list_dcim_media(&self.app).await.map_err(|e| anyhow!(e))?;
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let (absolute_path, ephemeral) = materialize_for_upload(&self.app, &item.uri)
                .await
                .map_err(|e| anyhow!(e))?;
            out.push(LocalCandidate {
                relative_path: media_item_relative_path(&item.relative_path, &item.display_name),
                absolute_path,
                size: item.size,
                mtime_ms: item.date_modified_ms,
                ephemeral,
            });
        }
        Ok(out)
    }
}

#[async_trait]
impl<R: Runtime> LocalMediaSource for AndroidDcimMediaSource<R> {
    async fn list_candidates(&self, binding: &Binding) -> anyhow::Result<Vec<LocalCandidate>> {
        #[cfg(target_os = "android")]
        {
            if matches!(binding.mode, sarca_sync::BindingMode::AutoUpload) {
                return self.list_dcim_via_mediastore().await;
            }
        }
        // FolderUpload/Sync (and AutoUpload on non-Android builds of this type,
        // which is never constructed by `state.rs`): plain filesystem walk.
        collect_fs_candidates(Path::new(&binding.local_path), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_relative_under_dcim() {
        assert_eq!(
            media_item_relative_path("DCIM/Camera/", "IMG_1.jpg"),
            "Camera/IMG_1.jpg"
        );
        assert_eq!(media_item_relative_path("DCIM/", "x.mp4"), "x.mp4");
    }
}
