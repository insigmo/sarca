//! Pluggable discovery of local upload candidates.
//!
//! Desktop / non-Android platforms always use [`FsMediaSource`], which walks
//! `binding.local_path` on disk. Android's Camera auto-upload instead lists
//! DCIM via MediaStore (see the `sarca-client` crate) so it can see files
//! that a raw filesystem walk cannot (e.g. `content://` scoped storage).

use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;

use crate::candidate::{collect_fs_candidates, LocalCandidate};
use crate::types::{Binding, BindingMode};

#[async_trait]
pub trait LocalMediaSource: Send + Sync {
    async fn list_candidates(&self, binding: &Binding) -> Result<Vec<LocalCandidate>>;
}

/// Default source: walks `binding.local_path` on the filesystem.
pub struct FsMediaSource;

#[async_trait]
impl LocalMediaSource for FsMediaSource {
    async fn list_candidates(&self, binding: &Binding) -> Result<Vec<LocalCandidate>> {
        let media_only = matches!(binding.mode, BindingMode::AutoUpload);
        let root = binding.local_path.clone();
        // WalkDir is blocking; keep it off the Tokio worker so Tauri IPC / UI
        // stay responsive during large gallery scans.
        tokio::task::spawn_blocking(move || collect_fs_candidates(Path::new(&root), media_only))
            .await
            .map_err(|e| anyhow::anyhow!("walk join error: {e}"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn fs_media_source_lists_media_only_for_auto_upload() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jpg"), b"x").unwrap();
        std::fs::write(dir.path().join("note.txt"), b"y").unwrap();
        let binding = Binding {
            id: "b1".into(),
            storage_id: Uuid::new_v4(),
            remote_root: "Camera".into(),
            local_path: dir.path().to_string_lossy().into(),
            mode: BindingMode::AutoUpload,
            enabled: true,
        };
        let got = FsMediaSource.list_candidates(&binding).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].relative_path, "a.jpg");
    }

    #[tokio::test]
    async fn fs_media_source_errors_on_missing_root() {
        let dir = tempfile::tempdir().unwrap();
        let binding = Binding {
            id: "b1".into(),
            storage_id: Uuid::new_v4(),
            remote_root: "Root".into(),
            local_path: dir.path().join("missing").to_string_lossy().into(),
            mode: BindingMode::FolderUpload,
            enabled: true,
        };
        assert!(FsMediaSource.list_candidates(&binding).await.is_err());
    }
}
