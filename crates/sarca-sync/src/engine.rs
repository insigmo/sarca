use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::{info, warn};
use walkdir::WalkDir;

use crate::{
    api::SarcaApi,
    hash::sha256_file,
    index::{mtime_ms_from_system, IndexEntry, LocalIndex},
    types::{Binding, BindingMode, SyncStatus},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    KeepLocal,
    KeepRemote,
    KeepBoth,
}

#[async_trait]
pub trait ConflictPrompt: Send + Sync {
    async fn ask(
        &self,
        binding_id: &str,
        relative_path: &str,
        local_hash: Option<&str>,
        remote_hash: Option<&str>,
    ) -> ConflictChoice;
}

/// Default prompt: keep both (safe, non-interactive).
pub struct KeepBothPrompt;

#[async_trait]
impl ConflictPrompt for KeepBothPrompt {
    async fn ask(
        &self,
        _binding_id: &str,
        _relative_path: &str,
        _local_hash: Option<&str>,
        _remote_hash: Option<&str>,
    ) -> ConflictChoice {
        ConflictChoice::KeepBoth
    }
}

#[derive(Clone)]
pub struct SyncEngineConfig {
    pub poll_interval: Duration,
    pub api: Arc<tokio::sync::RwLock<SarcaApi>>,
    pub data_dir: PathBuf,
}

pub struct SyncEngine {
    config: SyncEngineConfig,
    index: LocalIndex,
    prompt: Arc<dyn ConflictPrompt>,
    statuses: Arc<RwLock<Vec<SyncStatus>>>,
}

impl SyncEngine {
    pub fn open(config: SyncEngineConfig, prompt: Arc<dyn ConflictPrompt>) -> Result<Self> {
        let index_path = LocalIndex::default_path(&config.data_dir);
        let index = LocalIndex::open(&index_path)?;
        Ok(Self {
            config,
            index,
            prompt,
            statuses: Arc::new(RwLock::new(Vec::new())),
        })
    }

    pub async fn set_credentials(&self, base_url: String, access_token: String) {
        let mut api = self.config.api.write().await;
        *api = SarcaApi::new(base_url, access_token);
    }

    async fn api(&self) -> tokio::sync::RwLockReadGuard<'_, SarcaApi> {
        self.config.api.read().await
    }

    pub fn list_bindings(&self) -> Result<Vec<Binding>> {
        self.index.list_bindings()
    }

    pub fn upsert_binding(&self, binding: &Binding) -> Result<()> {
        self.index.upsert_binding(binding)
    }

    pub fn remove_binding(&self, id: &str) -> Result<()> {
        self.index.remove_binding(id)
    }

    pub async fn statuses(&self) -> Vec<SyncStatus> {
        self.statuses.read().await.clone()
    }

    /// Run one sync/auto-upload pass for all enabled bindings.
    pub async fn tick(&self) -> Result<()> {
        self.tick_filtered(|_| true).await
    }

    /// Like [`tick`], but only processes bindings for which `allow` returns true.
    pub async fn tick_filtered<F>(&self, allow: F) -> Result<()>
    where
        F: Fn(&Binding) -> bool,
    {
        let bindings = self.index.list_bindings()?;
        let mut statuses = Vec::new();
        for binding in bindings.into_iter().filter(|b| b.enabled && allow(b)) {
            let status = match self.sync_binding(&binding).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(binding = %binding.id, error = %e, "sync failed");
                    SyncStatus {
                        binding_id: binding.id.clone(),
                        cursor: self.index.get_cursor(&binding.id).unwrap_or(0),
                        last_error: Some(e.to_string()),
                        uploading: 0,
                        downloading: 0,
                        conflicts: self.index.conflict_count(&binding.id).unwrap_or(0),
                    }
                }
            };
            statuses.push(status);
        }
        *self.statuses.write().await = statuses;
        Ok(())
    }

    /// Continuous loop (desktop background / mobile foreground).
    pub async fn run_loop(self: Arc<Self>, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        loop {
            if let Err(e) = self.tick().await {
                warn!(error = %e, "sync tick error");
            }
            tokio::select! {
                _ = tokio::time::sleep(self.config.poll_interval) => {},
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    }

    async fn sync_binding(&self, binding: &Binding) -> Result<SyncStatus> {
        let mut uploading = 0usize;
        let mut downloading = 0usize;

        // First-time: pull snapshot if cursor is 0 and mode is Sync.
        let mut cursor = self.index.get_cursor(&binding.id)?;
        if cursor == 0 && matches!(binding.mode, BindingMode::Sync) {
            cursor = self.bootstrap_snapshot(binding).await?;
        }

        // Push local changes (both modes).
        uploading += self.push_local(binding).await?;

        if matches!(binding.mode, BindingMode::Sync) {
            downloading += self.pull_remote(binding, &mut cursor).await?;
            self.index.set_cursor(&binding.id, cursor)?;
        }

        Ok(SyncStatus {
            binding_id: binding.id.clone(),
            cursor,
            last_error: None,
            uploading,
            downloading,
            conflicts: self.index.conflict_count(&binding.id)?,
        })
    }

    async fn bootstrap_snapshot(&self, binding: &Binding) -> Result<i64> {
        info!(binding = %binding.id, "bootstrapping from snapshot");
        let snap = self.api().await.snapshot(binding.storage_id).await?;
        let root = PathBuf::from(&binding.local_path);
        for entry in snap.files {
            let rel = strip_remote_root(&entry.path, &binding.remote_root);
            let Some(rel) = rel else { continue };
            let local = root.join(&rel);
            if entry.is_file {
                if local.exists() {
                    let local_hash = sha256_file(&local).await.ok();
                    if local_hash.as_ref() != entry.content_hash.as_ref()
                        && entry.content_hash.is_some()
                        && local_hash.is_some()
                    {
                        self.index.add_conflict(
                            &binding.id,
                            &rel,
                            local_hash.as_deref(),
                            entry.content_hash.as_deref(),
                        )?;
                        continue;
                    }
                } else {
                    self.api()
                        .await
                        .download_to(binding.storage_id, &entry.path, &local)
                        .await?;
                }
                let meta = tokio::fs::metadata(&local).await?;
                let mtime = meta.modified().ok().map(mtime_ms_from_system).unwrap_or(0);
                let hash = sha256_file(&local)
                    .await
                    .ok()
                    .or(entry.content_hash.clone());
                self.index.upsert_entry(
                    &binding.id,
                    &IndexEntry {
                        relative_path: rel,
                        size: entry.size,
                        mtime_ms: mtime,
                        content_hash: hash,
                        remote_file_id: Some(entry.file_id),
                        last_cursor: snap.cursor,
                    },
                )?;
            } else {
                tokio::fs::create_dir_all(&local).await.ok();
            }
        }
        self.index.set_cursor(&binding.id, snap.cursor)?;
        Ok(snap.cursor)
    }

    async fn push_local(&self, binding: &Binding) -> Result<usize> {
        let root = PathBuf::from(&binding.local_path);
        if !root.exists() {
            tokio::fs::create_dir_all(&root).await?;
            return Ok(0);
        }
        let mut uploaded = 0usize;
        for file in WalkDir::new(&root).into_iter().filter_map(Result::ok) {
            if !file.file_type().is_file() {
                continue;
            }
            let path = file.path();
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            if rel.is_empty() {
                continue;
            }
            let meta = file
                .metadata()
                .with_context(|| format!("meta {}", path.display()))?;
            let mtime = meta.modified().ok().map(mtime_ms_from_system).unwrap_or(0);
            let size = meta.len() as i64;
            let existing = self.index.get_entry(&binding.id, &rel)?;
            let needs_hash = existing
                .as_ref()
                .is_none_or(|e| e.size != size || e.mtime_ms != mtime);
            if !needs_hash {
                continue;
            }
            let hash = sha256_file(path).await?;
            if existing.as_ref().and_then(|e| e.content_hash.as_ref()) == Some(&hash) {
                // Touch index mtime/size only.
                if let Some(mut e) = existing {
                    e.size = size;
                    e.mtime_ms = mtime;
                    self.index.upsert_entry(&binding.id, &e)?;
                }
                continue;
            }

            // Conflict if remote side also differs and we are in Sync mode.
            if matches!(binding.mode, BindingMode::Sync) {
                if let Some(prev) = &existing {
                    if prev.content_hash.as_ref() != Some(&hash) && prev.remote_file_id.is_some() {
                        // Local changed vs last synced; remote may also have changed — resolved in pull.
                    }
                }
            }

            let (parent, filename) = split_parent_name(&rel);
            let remote_parent = join_remote(&binding.remote_root, &parent);
            self.ensure_remote_parents(binding, &parent).await?;
            self.api()
                .await
                .upload_file(
                    binding.storage_id,
                    &remote_parent,
                    &filename,
                    path,
                    Some(mtime),
                    Some(&hash),
                )
                .await?;
            self.index.upsert_entry(
                &binding.id,
                &IndexEntry {
                    relative_path: rel,
                    size,
                    mtime_ms: mtime,
                    content_hash: Some(hash),
                    remote_file_id: existing.and_then(|e| e.remote_file_id),
                    last_cursor: self.index.get_cursor(&binding.id)?,
                },
            )?;
            uploaded += 1;
        }

        // Detect local deletes for Sync mode.
        if matches!(binding.mode, BindingMode::Sync) {
            let indexed = self.index.list_entry_paths(&binding.id)?;
            for rel in indexed {
                let local = root.join(&rel);
                if !local.exists() {
                    let remote = join_remote(&binding.remote_root, &rel);
                    self.api()
                        .await
                        .delete_remote(binding.storage_id, &remote)
                        .await?;
                    self.index.delete_entry(&binding.id, &rel)?;
                }
            }
        }
        Ok(uploaded)
    }

    async fn pull_remote(&self, binding: &Binding, cursor: &mut i64) -> Result<usize> {
        let mut downloaded = 0usize;
        let root = PathBuf::from(&binding.local_path);
        loop {
            let page = self
                .api()
                .await
                .changelog(binding.storage_id, *cursor, 500)
                .await?;
            for ev in &page.events {
                let Some(rel) = strip_remote_root(&ev.path, &binding.remote_root) else {
                    continue;
                };
                let local = root.join(&rel);
                match ev.op.as_str() {
                    "delete" => {
                        if local.exists() {
                            if local.is_dir() {
                                tokio::fs::remove_dir_all(&local).await.ok();
                            } else {
                                tokio::fs::remove_file(&local).await.ok();
                            }
                        }
                        self.index.delete_entry(&binding.id, &rel)?;
                        self.index.clear_conflict(&binding.id, &rel)?;
                    }
                    "upsert" => {
                        if !ev.is_file {
                            tokio::fs::create_dir_all(&local).await.ok();
                            continue;
                        }
                        let existing = self.index.get_entry(&binding.id, &rel)?;
                        if local.exists() {
                            let local_hash = sha256_file(&local).await.ok();
                            if local_hash.as_ref() != ev.content_hash.as_ref()
                                && existing.as_ref().and_then(|e| e.content_hash.as_ref())
                                    != local_hash.as_ref()
                                && ev.content_hash.is_some()
                            {
                                let choice = self
                                    .prompt
                                    .ask(
                                        &binding.id,
                                        &rel,
                                        local_hash.as_deref(),
                                        ev.content_hash.as_deref(),
                                    )
                                    .await;
                                match choice {
                                    ConflictChoice::KeepLocal => {
                                        // Re-upload local in next push; skip download.
                                        self.index.clear_conflict(&binding.id, &rel)?;
                                        continue;
                                    }
                                    ConflictChoice::KeepBoth => {
                                        let conflict_name = conflict_path(&local);
                                        tokio::fs::rename(&local, &conflict_name).await.ok();
                                    }
                                    ConflictChoice::KeepRemote => {}
                                }
                            } else if local_hash.as_ref() == ev.content_hash.as_ref() {
                                // Already in sync.
                                if let Some(mut e) = existing {
                                    e.remote_file_id = ev.file_id;
                                    e.last_cursor = ev.id;
                                    self.index.upsert_entry(&binding.id, &e)?;
                                }
                                continue;
                            }
                        }
                        self.api()
                            .await
                            .download_to(binding.storage_id, &ev.path, &local)
                            .await?;
                        let meta = tokio::fs::metadata(&local).await?;
                        let mtime = meta.modified().ok().map(mtime_ms_from_system).unwrap_or(0);
                        let hash = sha256_file(&local)
                            .await
                            .ok()
                            .or_else(|| ev.content_hash.clone());
                        self.index.upsert_entry(
                            &binding.id,
                            &IndexEntry {
                                relative_path: rel.clone(),
                                size: ev.size.unwrap_or(meta.len() as i64),
                                mtime_ms: mtime,
                                content_hash: hash,
                                remote_file_id: ev.file_id,
                                last_cursor: ev.id,
                            },
                        )?;
                        self.index.clear_conflict(&binding.id, &rel)?;
                        downloaded += 1;
                    }
                    other => warn!(op = other, "unknown changelog op"),
                }
            }
            if page.next_cursor > *cursor {
                *cursor = page.next_cursor;
            }
            if !page.has_more {
                break;
            }
        }
        Ok(downloaded)
    }

    async fn ensure_remote_parents(&self, binding: &Binding, parent: &str) -> Result<()> {
        if parent.is_empty() {
            return Ok(());
        }
        let mut built = String::new();
        for part in parent.split('/').filter(|p| !p.is_empty()) {
            let folder_parent = if built.is_empty() {
                binding.remote_root.trim_matches('/').to_owned()
            } else {
                join_remote(&binding.remote_root, &built)
            };
            self.api()
                .await
                .create_folder(binding.storage_id, &folder_parent, part)
                .await
                .ok();
            if built.is_empty() {
                built = part.to_owned();
            } else {
                built = format!("{built}/{part}");
            }
        }
        Ok(())
    }
}

fn strip_remote_root(path: &str, remote_root: &str) -> Option<String> {
    let path = path.trim_start_matches('/');
    let root = remote_root.trim().trim_matches('/');
    if root.is_empty() {
        return Some(path.trim_end_matches('/').to_owned());
    }
    let prefix = format!("{root}/");
    if path == root || path == format!("{root}/") {
        return Some(String::new());
    }
    path.strip_prefix(&prefix)
        .map(|s| s.trim_end_matches('/').to_owned())
}

fn join_remote(remote_root: &str, rel: &str) -> String {
    let root = remote_root.trim().trim_matches('/');
    let rel = rel.trim_matches('/');
    if root.is_empty() {
        rel.to_owned()
    } else if rel.is_empty() {
        root.to_owned()
    } else {
        format!("{root}/{rel}")
    }
}

fn split_parent_name(rel: &str) -> (String, String) {
    match rel.rsplit_once('/') {
        Some((p, n)) => (p.to_owned(), n.to_owned()),
        None => (String::new(), rel.to_owned()),
    }
}

fn conflict_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    let name = match ext {
        Some(e) => format!("{stem} (conflict).{e}"),
        None => format!("{stem} (conflict)"),
    };
    parent.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_root_works() {
        assert_eq!(strip_remote_root("a/b.txt", ""), Some("a/b.txt".into()));
        assert_eq!(
            strip_remote_root("docs/a.txt", "docs"),
            Some("a.txt".into())
        );
        assert_eq!(strip_remote_root("other/a.txt", "docs"), None);
    }

    #[test]
    fn conflict_name() {
        let p = conflict_path(Path::new("/tmp/foo.txt"));
        assert_eq!(p.file_name().unwrap(), "foo (conflict).txt");
    }
}
