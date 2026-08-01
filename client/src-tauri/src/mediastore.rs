//! Android MediaStore bridge: DCIM discovery + upload materialization.
//!
//! Desktop / non-Android platforms never construct [`AndroidDcimMediaSource`]
//! (see `state.rs`); this module still compiles there so `cargo test --lib`
//! covers the platform-independent path-join helper.

use std::path::Path;

use async_trait::async_trait;
use sarca_sync::{
    collect_fs_candidates, strip_dcim_prefix, Binding, LocalCandidate, LocalMediaSource,
};
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
                let handle =
                    api.register_android_plugin("app.sarca.client.mediastore", "MediaStorePlugin")?;
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
    /// Absolute filesystem path (MediaStore `DATA` column), present only when
    /// Kotlin could confirm the file is directly readable. When set, the Rust
    /// side can use it as the upload source without materializing every item
    /// via `materializeForUpload` first (see `list_dcim_via_mediastore`).
    #[serde(default)]
    pub path: Option<String>,
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

    /// Lists DCIM candidates without materializing every item up front: when
    /// MediaStore reports a directly-readable `DATA` path, that path is used
    /// as-is (`ephemeral: false`, no cache copy). `materializeForUpload` is
    /// only invoked for items where Kotlin could not confirm a readable path
    /// (e.g. scoped-storage `content://`-only entries on newer Android).
    ///
    /// Individual materialize failures are logged and skipped — one corrupt
    /// or since-deleted item should not abort the whole listing. Only errors
    /// out if the MediaStore query itself failed, or if every item in a
    /// non-empty list failed to produce a usable candidate.
    #[cfg(target_os = "android")]
    async fn list_dcim_via_mediastore(&self) -> anyhow::Result<Vec<LocalCandidate>> {
        use anyhow::anyhow;

        let items = list_dcim_media(&self.app).await.map_err(|e| anyhow!(e))?;
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let total = items.len();
        let mut out = Vec::with_capacity(total);
        for item in items {
            let relative_path = media_item_relative_path(&item.relative_path, &item.display_name);
            let usable_path = item.path.as_deref().filter(|p| !p.trim().is_empty());
            let candidate = if let Some(path) = usable_path {
                Some(LocalCandidate {
                    relative_path,
                    absolute_path: std::path::PathBuf::from(path),
                    size: item.size,
                    mtime_ms: item.date_modified_ms,
                    ephemeral: false,
                })
            } else {
                match materialize_for_upload(&self.app, &item.uri).await {
                    Ok((absolute_path, ephemeral)) => Some(LocalCandidate {
                        relative_path,
                        absolute_path,
                        size: item.size,
                        mtime_ms: item.date_modified_ms,
                        ephemeral,
                    }),
                    Err(e) => {
                        tracing::warn!(uri = %item.uri, error = %e, "materialize failed, skipping item");
                        None
                    }
                }
            };
            if let Some(c) = candidate {
                out.push(c);
            }
        }
        if out.is_empty() {
            anyhow::bail!("failed to materialize any of {total} MediaStore item(s)");
        }
        Ok(out)
    }
}

/// True when `local_path` is DCIM itself or lives under a DCIM subfolder
/// (e.g. `.../DCIM/Camera`) — the only place MediaStore can see files a raw
/// filesystem walk cannot (scoped-storage `content://` entries). AutoUpload
/// bindings pointed elsewhere (e.g. a user-picked non-DCIM folder) must fall
/// back to a plain walk instead: MediaStore only indexes DCIM, so treating
/// any AutoUpload binding as MediaStore-backed would silently show zero
/// files for those.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn is_dcim_local_path(local_path: &str) -> bool {
    dcim_path_segments(local_path).is_some()
}

/// Splits `local_path` into (nothing) when it doesn't contain a `DCIM` path
/// segment, or `Some(subfolder)` when it does — `subfolder` is empty for the
/// DCIM root itself, or the path underneath it (e.g. `Camera`,
/// `Camera/2024`) for a binding rooted at a DCIM subfolder.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn dcim_path_segments(local_path: &str) -> Option<String> {
    let trimmed = local_path.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    let idx = segments.iter().position(|&s| s == "DCIM")?;
    Some(segments[idx + 1..].join("/"))
}

/// Restricts MediaStore results to a DCIM subfolder binding (e.g.
/// `DCIM/Camera`) so a Camera-only binding doesn't upload the whole DCIM
/// tree (Screenshots, WhatsApp, etc.). No-op when the binding is rooted at
/// DCIM itself.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn filter_to_dcim_subfolder(
    candidates: Vec<LocalCandidate>,
    local_path: &str,
) -> Vec<LocalCandidate> {
    let Some(subfolder) = dcim_path_segments(local_path).filter(|s| !s.is_empty()) else {
        return candidates;
    };
    let prefix = format!("{subfolder}/");
    candidates
        .into_iter()
        .filter(|c| c.relative_path.starts_with(&prefix))
        .collect()
}

#[async_trait]
impl<R: Runtime> LocalMediaSource for AndroidDcimMediaSource<R> {
    async fn list_candidates(&self, binding: &Binding) -> anyhow::Result<Vec<LocalCandidate>> {
        let media_only = matches!(binding.mode, sarca_sync::BindingMode::AutoUpload);
        #[cfg(target_os = "android")]
        {
            if media_only && is_dcim_local_path(&binding.local_path) {
                let candidates = self.list_dcim_via_mediastore().await?;
                return Ok(filter_to_dcim_subfolder(candidates, &binding.local_path));
            }
        }
        // FolderUpload/Sync, an AutoUpload binding not rooted at DCIM, and
        // AutoUpload on non-Android builds of this type (never constructed
        // by `state.rs`): plain filesystem walk.
        collect_fs_candidates(Path::new(&binding.local_path), media_only)
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

    #[test]
    fn dcim_local_path_matches_common_forms() {
        assert!(is_dcim_local_path("/storage/emulated/0/DCIM"));
        assert!(is_dcim_local_path("/storage/emulated/0/DCIM/"));
        assert!(is_dcim_local_path("/sdcard/DCIM"));
        assert!(is_dcim_local_path("DCIM"));
        // Subfolders under DCIM (e.g. Camera) are scoped-storage-only too —
        // a raw filesystem walk sees nothing there, so these must also route
        // through MediaStore rather than falling through to WalkDir.
        assert!(is_dcim_local_path("/storage/emulated/0/DCIM/Camera"));
        assert!(is_dcim_local_path("/storage/emulated/0/DCIM/Camera/"));
        assert!(is_dcim_local_path("/sdcard/DCIM/Camera/2024"));
        assert!(!is_dcim_local_path("/storage/emulated/0/Pictures"));
        assert!(!is_dcim_local_path(""));
        assert!(!is_dcim_local_path("/storage/emulated/0/Download"));
        // A folder that merely starts with "DCIM" as a substring (not a full
        // path segment) must not match.
        assert!(!is_dcim_local_path("/storage/emulated/0/DCIMBackup"));
    }

    #[test]
    fn dcim_path_segments_extracts_subfolder() {
        assert_eq!(
            dcim_path_segments("/storage/emulated/0/DCIM").as_deref(),
            Some("")
        );
        assert_eq!(
            dcim_path_segments("/storage/emulated/0/DCIM/Camera").as_deref(),
            Some("Camera")
        );
        assert_eq!(
            dcim_path_segments("/storage/emulated/0/DCIM/Camera/2024/").as_deref(),
            Some("Camera/2024")
        );
        assert_eq!(dcim_path_segments("/storage/emulated/0/Pictures"), None);
    }

    fn candidate(relative_path: &str) -> LocalCandidate {
        LocalCandidate {
            relative_path: relative_path.to_string(),
            absolute_path: std::path::PathBuf::from("/tmp").join(relative_path),
            size: 1,
            mtime_ms: 0,
            ephemeral: false,
        }
    }

    #[test]
    fn filter_to_dcim_subfolder_keeps_everything_at_dcim_root() {
        let candidates = vec![candidate("Camera/a.jpg"), candidate("Screenshots/b.jpg")];
        let filtered = filter_to_dcim_subfolder(candidates.clone(), "/storage/emulated/0/DCIM");
        assert_eq!(filtered.len(), candidates.len());
    }

    #[test]
    fn filter_to_dcim_subfolder_restricts_to_camera_only() {
        let candidates = vec![
            candidate("Camera/a.jpg"),
            candidate("Screenshots/b.jpg"),
            candidate("Camera/nested/c.jpg"),
        ];
        let filtered = filter_to_dcim_subfolder(candidates, "/storage/emulated/0/DCIM/Camera");
        let paths: Vec<_> = filtered.iter().map(|c| c.relative_path.as_str()).collect();
        assert_eq!(paths, vec!["Camera/a.jpg", "Camera/nested/c.jpg"]);
    }
}
