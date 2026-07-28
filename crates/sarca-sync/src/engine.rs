use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::{
    api::SarcaApi,
    candidate::LocalCandidate,
    hash::sha256_file,
    index::{mtime_ms_from_system, IndexEntry, LocalIndex},
    media_source::LocalMediaSource,
    scheduler::BindingScheduler,
    transfer::{TransferDirection, TransferQueue, TransferQueueSnapshot},
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
    /// Discovers local upload candidates for a binding. Defaults to
    /// [`FsMediaSource`] (filesystem walk); Android's Camera auto-upload
    /// overrides this with a MediaStore-backed source.
    pub media_source: Arc<dyn LocalMediaSource>,
}

pub struct SyncEngine {
    config: SyncEngineConfig,
    index: LocalIndex,
    prompt: Arc<dyn ConflictPrompt>,
    statuses: Arc<RwLock<Vec<SyncStatus>>>,
    transfers: Arc<RwLock<TransferQueue>>,
    /// Runs enabled bindings concurrently (per-id skip-when-busy, max 2 in flight).
    scheduler: BindingScheduler,
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
            transfers: Arc::new(RwLock::new(TransferQueue::default())),
            scheduler: BindingScheduler::new(2),
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
        self.index.remove_binding(id)?;
        self.clear_status(id);
        self.clear_transfers(id);
        Ok(())
    }

    pub fn set_binding_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.index.set_binding_enabled(id, enabled)?;
        if !enabled {
            self.clear_status(id);
            self.clear_transfers(id);
        }
        Ok(())
    }

    /// Best-effort immediate removal of a binding's status entry, e.g. right
    /// after disabling/removing it so a stale error banner doesn't linger in
    /// the UI until the next tick. This is synchronous (`statuses` is an
    /// async `RwLock`), so it uses `try_write`: if a tick currently holds the
    /// lock, the entry is instead pruned by `tick_filtered`'s own retain pass
    /// once that tick completes — never left stuck forever.
    fn clear_status(&self, id: &str) {
        if let Ok(mut guard) = self.statuses.try_write() {
            guard.retain(|s| s.binding_id != id);
        }
    }

    fn clear_transfers(&self, id: &str) {
        if let Ok(mut guard) = self.transfers.try_write() {
            guard.clear_binding(id);
        }
    }

    pub async fn statuses(&self) -> Vec<SyncStatus> {
        self.statuses.read().await.clone()
    }

    pub async fn transfer_queue(&self) -> TransferQueueSnapshot {
        self.transfers.read().await.snapshot()
    }

    async fn transfer_begin(
        &self,
        binding_id: &str,
        direction: TransferDirection,
        relative_path: &str,
        size: Option<i64>,
    ) -> String {
        self.transfers
            .write()
            .await
            .begin(binding_id, direction, relative_path, size)
    }

    async fn transfer_complete(&self, id: &str) {
        self.transfers.write().await.complete(id);
    }

    async fn transfer_abandon(&self, id: &str) {
        self.transfers.write().await.abandon(id);
    }

    /// Promote a previously-enqueued Waiting transfer to Active. Falls back
    /// to [`begin`](TransferQueue::begin) if there was no waiting id (e.g.
    /// the queue was over [`crate::transfer::MAX_WAITING`] at enqueue time).
    async fn transfer_promote(
        &self,
        waiting_id: Option<&str>,
        binding_id: &str,
        direction: TransferDirection,
        relative_path: &str,
        size: Option<i64>,
    ) -> String {
        let mut queue = self.transfers.write().await;
        if let Some(id) = waiting_id {
            if queue.promote(id) {
                return id.to_owned();
            }
        }
        queue.begin(binding_id, direction, relative_path, size)
    }

    /// Run one sync/auto-upload pass for all enabled bindings.
    pub async fn tick(&self) -> Result<()> {
        self.tick_filtered(|_| true).await
    }

    /// Like [`tick`], but only processes bindings for which `allow` returns true.
    /// Auto-upload bindings run before two-way sync so a huge sync folder cannot
    /// starve Camera uploads for the whole poll interval.
    ///
    /// Bindings run concurrently via [`BindingScheduler`]: a binding already
    /// in flight (e.g. from an overlapping call) is skipped rather than
    /// serialized behind a global lock, so Camera auto-upload and folder sync
    /// no longer block each other.
    pub async fn tick_filtered<F>(&self, allow: F) -> Result<()>
    where
        F: Fn(&Binding) -> bool,
    {
        let mut bindings: Vec<Binding> = self
            .index
            .list_bindings()?
            .into_iter()
            .filter(|b| b.enabled && allow(b))
            .collect();
        bindings.sort_by_key(|b| match b.mode {
            BindingMode::AutoUpload | BindingMode::FolderUpload => 0,
            BindingMode::Sync => 1,
        });

        // Placeholders are seeded lazily, only once the scheduler has
        // actually accepted a binding's run (inside the closure passed to
        // `run`, below) — never up front for every binding. Seeding up
        // front would clobber the still-valid status of a binding that
        // gets skipped (`None`, already in flight from an overlapping
        // tick), wiping its `last_error`/counts even though it never ran.
        let futs = bindings.into_iter().map(|binding| async move {
            self.scheduler
                .run(&binding.id, || async {
                    // Now that the scheduler has committed to running this
                    // binding, publish an in-progress placeholder so the UI
                    // is not blank for long ticks and so per-file progress
                    // updates (in `push_local`) have a status entry to
                    // update while other bindings run concurrently.
                    {
                        let placeholder = SyncStatus {
                            binding_id: binding.id.clone(),
                            cursor: self.index.get_cursor(&binding.id).unwrap_or(0),
                            last_error: None,
                            uploading: 0,
                            downloading: 0,
                            conflicts: self.index.conflict_count(&binding.id).unwrap_or(0),
                        };
                        let mut guard = self.statuses.write().await;
                        match guard.iter_mut().find(|s| s.binding_id == binding.id) {
                            Some(existing) => *existing = placeholder,
                            None => guard.push(placeholder),
                        }
                    }

                    match self.sync_binding(&binding).await {
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
                    }
                })
                .await
        });
        let results = futures::future::join_all(futs).await;

        // Merge completed statuses back in; bindings that were skipped
        // (`None`, already in flight elsewhere) keep their existing status.
        let mut guard = self.statuses.write().await;
        for status in results.into_iter().flatten() {
            match guard.iter_mut().find(|s| s.binding_id == status.binding_id) {
                Some(existing) => *existing = status,
                None => guard.push(status),
            }
        }

        // Drop statuses for bindings that no longer exist or were disabled
        // since they were last synced — otherwise a removed/disabled
        // binding's stale `last_error` keeps showing an error banner in the
        // UI forever (the binding never runs again to clear it). Re-read
        // the authoritative list here (not the `allow`-filtered `bindings`
        // above) so a binding merely skipped by `allow` this tick — e.g.
        // Wi‑Fi‑only throttling — keeps its status.
        let current = self.index.list_bindings()?;
        guard.retain(|s| current.iter().any(|b| b.id == s.binding_id && b.enabled));
        Ok(())
    }

    /// Runs a single binding through the same filtered pipeline (e.g. for a
    /// UI-triggered "sync now" on one binding) without affecting others.
    pub async fn tick_binding<F>(&self, binding_id: &str, allow: F) -> Result<()>
    where
        F: Fn(&Binding) -> bool,
    {
        self.tick_filtered(|b| b.id == binding_id && allow(b)).await
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
                    let tid = self
                        .transfer_begin(
                            &binding.id,
                            TransferDirection::Download,
                            &rel,
                            Some(entry.size),
                        )
                        .await;
                    let dl = self
                        .api()
                        .await
                        .download_to(binding.storage_id, &entry.path, &local)
                        .await;
                    if let Err(e) = dl {
                        self.transfer_abandon(&tid).await;
                        return Err(e);
                    }
                    self.transfer_complete(&tid).await;
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
        let upload_only = binding.mode.is_upload_only();

        let candidates = self.config.media_source.list_candidates(binding).await?;
        let pending = {
            let mut queue = self.transfers.write().await;
            select_pending_uploads(&self.index, &binding.id, &candidates, &mut queue)?
        };

        let mut uploaded = 0usize;
        for (candidate, waiting_id) in pending {
            let LocalCandidate {
                relative_path: rel,
                absolute_path: path,
                size,
                mtime_ms: mtime,
                ephemeral,
            } = candidate;
            let existing = self.index.get_entry(&binding.id, &rel)?;

            let tid = self
                .transfer_promote(
                    waiting_id.as_deref(),
                    &binding.id,
                    TransferDirection::Upload,
                    &rel,
                    Some(size),
                )
                .await;

            let hash = match sha256_file(&path).await {
                Ok(h) => h,
                Err(e) => {
                    self.transfer_abandon(&tid).await;
                    return Err(e).with_context(|| format!("hash {}", path.display()));
                }
            };

            let (parent, filename) = split_parent_name(&rel);
            let remote_parent = join_remote(&binding.remote_root, &parent);
            if let Err(e) = self.ensure_remote_parents(binding, &parent).await {
                self.transfer_abandon(&tid).await;
                return Err(e);
            }
            let upload_result = self
                .api()
                .await
                .upload_file(
                    binding.storage_id,
                    &remote_parent,
                    &filename,
                    &path,
                    Some(mtime),
                    Some(&hash),
                )
                .await;
            if let Err(e) = upload_result {
                self.transfer_abandon(&tid).await;
                return Err(e).with_context(|| {
                    format!(
                        "upload {} → {}/{}",
                        path.display(),
                        remote_parent,
                        filename
                    )
                });
            }
            self.transfer_complete(&tid).await;
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
            if ephemeral {
                tokio::fs::remove_file(&path).await.ok();
            }
            uploaded += 1;
            // Live progress for long Camera / folder uploads (Telegram is slow).
            if upload_only {
                let mut guard = self.statuses.write().await;
                if let Some(s) = guard.iter_mut().find(|s| s.binding_id == binding.id) {
                    s.uploading = uploaded;
                    s.last_error = None;
                }
            }
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
                        let tid = self
                            .transfer_begin(
                                &binding.id,
                                TransferDirection::Download,
                                &rel,
                                ev.size,
                            )
                            .await;
                        let dl = self
                            .api()
                            .await
                            .download_to(binding.storage_id, &ev.path, &local)
                            .await;
                        if let Err(e) = dl {
                            self.transfer_abandon(&tid).await;
                            return Err(e);
                        }
                        self.transfer_complete(&tid).await;
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

/// Filters `candidates` down to those whose size/mtime differ from the
/// index (or have no index entry yet), enqueuing each as Waiting in
/// `queue`. The paired `Option<String>` is the waiting transfer id, or
/// `None` if the queue was over [`crate::transfer::MAX_WAITING`].
pub fn select_pending_uploads(
    index: &LocalIndex,
    binding_id: &str,
    candidates: &[LocalCandidate],
    queue: &mut TransferQueue,
) -> Result<Vec<(LocalCandidate, Option<String>)>> {
    let mut out = Vec::new();
    for c in candidates {
        let existing = index.get_entry(binding_id, &c.relative_path)?;
        let needs = existing
            .as_ref()
            .is_none_or(|e| e.size != c.size || e.mtime_ms != c.mtime_ms);
        if !needs {
            continue;
        }
        let tid = queue.enqueue_waiting(
            binding_id,
            TransferDirection::Upload,
            &c.relative_path,
            Some(c.size),
        );
        out.push((c.clone(), tid));
    }
    Ok(out)
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
    use crate::candidate::{is_media_file, LocalCandidate};
    use crate::transfer::TransferStatus;

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

    #[test]
    fn media_filter_accepts_photos_and_videos() {
        assert!(is_media_file(Path::new("/home/beta/Pictures/cat.jpg")));
        assert!(is_media_file(Path::new("x.PNG")));
        assert!(is_media_file(Path::new("clip.MP4")));
        assert!(is_media_file(Path::new("a.webp")));
        assert!(!is_media_file(Path::new("/home/beta/Pictures/index.html")));
        assert!(!is_media_file(Path::new("notes.txt")));
        assert!(!is_media_file(Path::new("noext")));
    }

    #[test]
    fn upload_only_modes() {
        assert!(BindingMode::AutoUpload.is_upload_only());
        assert!(BindingMode::FolderUpload.is_upload_only());
        assert!(!BindingMode::Sync.is_upload_only());
    }

    #[test]
    fn engine_set_binding_enabled_updates_flag() {
        let dir = tempfile::tempdir().unwrap();
        let engine = SyncEngine::open(
            SyncEngineConfig {
                poll_interval: Duration::from_secs(30),
                api: Arc::new(tokio::sync::RwLock::new(SarcaApi::new(
                    "http://127.0.0.1",
                    "",
                ))),
                data_dir: dir.path().to_path_buf(),
                media_source: Arc::new(crate::media_source::FsMediaSource),
            },
            Arc::new(KeepBothPrompt),
        )
        .unwrap();
        let id = "cam".to_string();
        engine
            .upsert_binding(&Binding {
                id: id.clone(),
                storage_id: uuid::Uuid::new_v4(),
                remote_root: "Camera".into(),
                local_path: dir.path().join("pics").to_string_lossy().into(),
                mode: BindingMode::AutoUpload,
                enabled: true,
            })
            .unwrap();
        engine.set_binding_enabled(&id, false).unwrap();
        let b = engine
            .list_bindings()
            .unwrap()
            .into_iter()
            .find(|b| b.id == id)
            .unwrap();
        assert!(!b.enabled);
    }

    fn test_engine(dir: &std::path::Path) -> SyncEngine {
        SyncEngine::open(
            SyncEngineConfig {
                poll_interval: Duration::from_secs(30),
                api: Arc::new(tokio::sync::RwLock::new(SarcaApi::new(
                    "http://127.0.0.1",
                    "",
                ))),
                data_dir: dir.to_path_buf(),
                media_source: Arc::new(crate::media_source::FsMediaSource),
            },
            Arc::new(KeepBothPrompt),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn disabling_binding_clears_its_status_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path());
        let id = "cam".to_string();
        engine
            .upsert_binding(&Binding {
                id: id.clone(),
                storage_id: uuid::Uuid::new_v4(),
                remote_root: "Camera".into(),
                local_path: dir.path().join("pics").to_string_lossy().into(),
                mode: BindingMode::AutoUpload,
                enabled: true,
            })
            .unwrap();
        engine.tick().await.unwrap();
        assert!(
            engine
                .statuses()
                .await
                .iter()
                .any(|s| s.binding_id == id),
            "status should exist after a successful tick"
        );

        engine.set_binding_enabled(&id, false).unwrap();
        assert!(
            engine
                .statuses()
                .await
                .iter()
                .all(|s| s.binding_id != id),
            "disabling a binding must clear its status so UI error banners clear"
        );
    }

    #[tokio::test]
    async fn removing_binding_clears_its_status_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path());
        let id = "cam".to_string();
        engine
            .upsert_binding(&Binding {
                id: id.clone(),
                storage_id: uuid::Uuid::new_v4(),
                remote_root: "Camera".into(),
                local_path: dir.path().join("pics").to_string_lossy().into(),
                mode: BindingMode::AutoUpload,
                enabled: true,
            })
            .unwrap();
        engine.tick().await.unwrap();
        assert!(engine.statuses().await.iter().any(|s| s.binding_id == id));

        engine.remove_binding(&id).unwrap();
        assert!(
            engine
                .statuses()
                .await
                .iter()
                .all(|s| s.binding_id != id),
            "removing a binding must clear its status"
        );
    }

    #[tokio::test]
    async fn tick_prunes_stale_status_for_binding_removed_since_last_tick() {
        // Simulates the case where the fast `clear_status` best-effort path
        // was skipped (e.g. lock contention) — the next `tick` must still
        // prune it via the authoritative retain pass.
        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path());
        let cam_id = "cam".to_string();
        let folder_id = "folder".to_string();
        for (id, mode) in [
            (cam_id.clone(), BindingMode::AutoUpload),
            (folder_id.clone(), BindingMode::FolderUpload),
        ] {
            engine
                .upsert_binding(&Binding {
                    id: id.clone(),
                    storage_id: uuid::Uuid::new_v4(),
                    remote_root: "Root".into(),
                    local_path: dir.path().join(&id).to_string_lossy().into(),
                    mode,
                    enabled: true,
                })
                .unwrap();
        }
        engine.tick().await.unwrap();
        assert_eq!(engine.statuses().await.len(), 2);

        // Remove directly via the index to bypass `clear_status`.
        engine.index.remove_binding(&cam_id).unwrap();
        engine.tick().await.unwrap();

        let statuses = engine.statuses().await;
        assert!(
            statuses.iter().all(|s| s.binding_id != cam_id),
            "next tick must prune status for a binding removed out from under it"
        );
        assert!(
            statuses.iter().any(|s| s.binding_id == folder_id),
            "surviving binding must keep its status"
        );
    }

    #[test]
    fn select_pending_uploads_marks_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let idx = LocalIndex::open(&dir.path().join("i.sqlite")).unwrap();
        let id = "b1";
        idx.upsert_binding(&Binding {
            id: id.to_string(),
            storage_id: uuid::Uuid::new_v4(),
            remote_root: "Camera".into(),
            local_path: dir.path().join("pics").to_string_lossy().into(),
            mode: BindingMode::AutoUpload,
            enabled: true,
        })
        .unwrap();
        let c = LocalCandidate {
            relative_path: "a.jpg".into(),
            absolute_path: dir.path().join("a.jpg"),
            size: 3,
            mtime_ms: 1,
            ephemeral: false,
        };
        let mut q = TransferQueue::default();
        let pending = select_pending_uploads(&idx, id, &[c], &mut q).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].1.is_some());
        assert_eq!(q.snapshot().uploading, 1);
        assert_eq!(q.snapshot().items[0].status, TransferStatus::Waiting);
    }

    #[test]
    fn select_pending_uploads_skips_unchanged_entries() {
        let dir = tempfile::tempdir().unwrap();
        let idx = LocalIndex::open(&dir.path().join("i.sqlite")).unwrap();
        let id = "b1";
        idx.upsert_binding(&Binding {
            id: id.to_string(),
            storage_id: uuid::Uuid::new_v4(),
            remote_root: "Camera".into(),
            local_path: dir.path().join("pics").to_string_lossy().into(),
            mode: BindingMode::AutoUpload,
            enabled: true,
        })
        .unwrap();
        idx.upsert_entry(
            id,
            &IndexEntry {
                relative_path: "a.jpg".into(),
                size: 3,
                mtime_ms: 1,
                content_hash: Some("abc".into()),
                remote_file_id: None,
                last_cursor: 0,
            },
        )
        .unwrap();
        let c = LocalCandidate {
            relative_path: "a.jpg".into(),
            absolute_path: dir.path().join("a.jpg"),
            size: 3,
            mtime_ms: 1,
            ephemeral: false,
        };
        let mut q = TransferQueue::default();
        let pending = select_pending_uploads(&idx, id, &[c], &mut q).unwrap();
        assert!(pending.is_empty());
        assert_eq!(q.snapshot().uploading, 0);
    }
}
