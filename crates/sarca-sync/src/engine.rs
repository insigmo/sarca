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
    index::{mtime_ms_from_system, now_ms, IndexEntry, LocalIndex, UploadFailure},
    media_source::LocalMediaSource,
    scheduler::BindingScheduler,
    transfer::{TransferDirection, TransferQueue, TransferQueueSnapshot},
    types::{Binding, BindingMode, SyncStatus},
};

/// How many files one binding uploads at the same time. One: overlapping files
/// does not actually buy throughput here, because the server funnels every file
/// through the same per-bot-token Telegram send gate. Asking for more only
/// pushes that gate into flood control, whose backoff is far more expensive than
/// the round trips the overlap was meant to hide, and holds one spool file per
/// in-flight upload on the server's disk. Chunks within a file are sequential
/// regardless.
const UPLOAD_PARALLELISM: usize = 1;

/// Retry delay after a file's upload fails, indexed by consecutive failure
/// count: 1 min, 5 min, 30 min, 1 h, then 6 h forever.
///
/// The point is head-of-line isolation, not politeness. Before this ladder a
/// file that could never upload (too large for Telegram, unreadable, rejected
/// by the server) was re-attempted first on every single tick, and since a
/// failure aborted the whole batch, every file behind it was starved forever.
/// Now the failure is recorded, the file drops out of the pending set until its
/// deadline passes, and the rest of the backlog moves.
const UPLOAD_BACKOFF_MS: [i64; 5] = [
    60_000,     // 1 min
    300_000,    // 5 min
    1_800_000,  // 30 min
    3_600_000,  // 1 h
    21_600_000, // 6 h
];

/// How long to wait before retrying a file that has failed `fail_count` times
/// in a row. Saturates at the last rung — a file is never given up on
/// permanently, because most causes (server down, no network, flood control,
/// full disk) are transient and clear on their own.
fn upload_backoff_ms(fail_count: i64) -> i64 {
    let rung = fail_count.max(1) as usize - 1;
    UPLOAD_BACKOFF_MS[rung.min(UPLOAD_BACKOFF_MS.len() - 1)]
}

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

/// Distinguishes a normal sync pass from the cheap remote-only pass used for
/// the foreground fast-poll cadence. See [`SyncEngine::tick_pull_only`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TickScope {
    /// Push local changes, then pull (for `Sync` bindings).
    Full,
    /// Skip `push_local` (and the local scan behind it) entirely; pull only.
    PullOnly,
}

struct PushLocalResult {
    uploaded: usize,
    scanned: usize,
    pending: usize,
    /// Files attempted this tick whose upload failed. Each is recorded in the
    /// index with a retry deadline; none of them aborts the batch.
    failed: usize,
    /// First failure's message, for the status banner.
    first_error: Option<String>,
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

    /// Clears the retry backoff so previously-failed files are reconsidered on
    /// the very next scan. Bound to the explicit "Upload now" action: a user
    /// who just fixed the cause (freed space, reconnected, deleted the bad
    /// file) should not have to wait out a deadline they cannot see. `None`
    /// clears every binding. Returns how many failure records were dropped.
    pub fn retry_failed_uploads(&self, binding_id: Option<&str>) -> Result<usize> {
        let ids: Vec<String> = match binding_id {
            Some(id) => vec![id.to_string()],
            None => self
                .index
                .list_bindings()?
                .into_iter()
                .map(|b| b.id)
                .collect(),
        };
        let mut cleared = 0usize;
        for id in ids {
            cleared += self.index.clear_upload_backoff(&id)?;
        }
        Ok(cleared)
    }

    /// Files currently held back by the retry backoff, worst first. Drives the
    /// UI hint that names what is stuck instead of silently doing nothing.
    pub fn failed_uploads(&self, binding_id: &str, limit: usize) -> Result<Vec<UploadFailure>> {
        self.index.list_upload_failures(binding_id, limit)
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

    // `cleanup_abandoned_ephemeral` used to live here, for the case where
    // `push_local` bailed out mid-batch and left candidates unattempted. There
    // is no such case any more: every candidate in the pending set is now
    // attempted, and `push_one` deletes its own ephemeral file on every exit
    // path, success or failure.

    /// Promote a previously-enqueued Waiting transfer to Active. Falls back
    /// to [`begin`](TransferQueue::begin) if `waiting_id` is no longer in the
    /// queue (e.g. the binding was cleared concurrently).
    async fn transfer_promote(
        &self,
        waiting_id: &str,
        binding_id: &str,
        direction: TransferDirection,
        relative_path: &str,
        size: Option<i64>,
    ) -> String {
        let mut queue = self.transfers.write().await;
        if queue.promote(waiting_id) {
            return waiting_id.to_owned();
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
        self.tick_filtered_scoped(allow, TickScope::Full).await
    }

    /// Remote-only counterpart to [`tick_filtered`]: restricted to two-way
    /// `Sync` bindings and skips `push_local` (and therefore the local
    /// filesystem/MediaStore scan) entirely. This is the cheap tick the
    /// foreground fast-poll cadence runs once [`remote_has_changes`] says
    /// there is something to pull — see `start_background_loop` in
    /// `client/src-tauri/src/state.rs` for why that split exists.
    pub async fn tick_pull_only(&self) -> Result<()> {
        self.tick_filtered_scoped(|b| matches!(b.mode, BindingMode::Sync), TickScope::PullOnly)
            .await
    }

    async fn tick_filtered_scoped<F>(&self, allow: F, scope: TickScope) -> Result<()>
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
                            ..Default::default()
                        };
                        let mut guard = self.statuses.write().await;
                        match guard.iter_mut().find(|s| s.binding_id == binding.id) {
                            Some(existing) => *existing = placeholder,
                            None => guard.push(placeholder),
                        }
                    }

                    match self.sync_binding_scoped(&binding, scope).await {
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
                                ..Default::default()
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

    /// One tiny changelog request (`limit=1`) per enabled `Sync` binding, no
    /// disk walk, no hashing — the "is it worth doing a real pass" check that
    /// keeps the 15s foreground poll cadence cheap enough for battery. See
    /// `start_background_loop` in `client/src-tauri/src/state.rs`: it calls
    /// this every tick and only runs [`tick_pull_only`](Self::tick_pull_only)
    /// (or, on the periodic local-scan tick, a full pass) when it returns
    /// `true`.
    pub async fn remote_has_changes(&self) -> Result<bool> {
        for binding in self.index.list_bindings()? {
            if !binding.enabled || !matches!(binding.mode, BindingMode::Sync) {
                continue;
            }
            let cursor = self.index.get_cursor(&binding.id)?;
            // Never bootstrapped: there is no cursor to diff against, so
            // treat it as "changed" rather than silently sitting at 0 until
            // something else happens to touch this binding.
            if cursor == 0 {
                return Ok(true);
            }
            let page = self
                .api()
                .await
                .changelog(binding.storage_id, cursor, 1)
                .await?;
            if !page.events.is_empty() {
                return Ok(true);
            }
        }
        Ok(false)
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

    /// `scope == PullOnly` skips `push_local` (see [`tick_pull_only`]);
    /// `Full` — what every existing caller through `sync_binding` got before
    /// this split — is unchanged.
    ///
    /// [`tick_pull_only`]: SyncEngine::tick_pull_only
    async fn sync_binding_scoped(&self, binding: &Binding, scope: TickScope) -> Result<SyncStatus> {
        let mut downloading = 0usize;

        // First-time: pull snapshot if cursor is 0 and mode is Sync.
        let mut cursor = self.index.get_cursor(&binding.id)?;
        if cursor == 0 && matches!(binding.mode, BindingMode::Sync) {
            cursor = self.bootstrap_snapshot(binding).await?;
        }

        // Push local changes (both modes). Individual upload failures are
        // recorded and skipped, not propagated — a file the server will not
        // take must not stop the pull below, which is what made a single bad
        // photo look like "nothing syncs at all". `PullOnly` skips this
        // (and the local scan behind it) entirely — the caller already knows
        // via `remote_has_changes` that this pass only needs to pull.
        let push = match scope {
            TickScope::Full => self.push_local(binding).await?,
            TickScope::PullOnly => PushLocalResult {
                uploaded: 0,
                scanned: 0,
                pending: 0,
                failed: 0,
                first_error: None,
            },
        };
        let (_scanned, pending, already_synced) =
            crate::types::scan_counters(push.scanned, push.pending);

        if matches!(binding.mode, BindingMode::Sync) {
            downloading += self.pull_remote(binding, &mut cursor).await?;
            self.index.set_cursor(&binding.id, cursor)?;
        }

        Ok(SyncStatus {
            binding_id: binding.id.clone(),
            cursor,
            last_error: push.first_error,
            uploading: push.uploaded,
            downloading,
            conflicts: self.index.conflict_count(&binding.id)?,
            scanned: push.scanned,
            pending,
            already_synced,
            failed: push.failed,
            deferred: self.index.upload_failure_count(&binding.id)?,
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

    async fn push_local(&self, binding: &Binding) -> Result<PushLocalResult> {
        let upload_only = binding.mode.is_upload_only();

        let candidates = self.config.media_source.list_candidates(binding).await?;
        // Filter against the SQLite-backed index *before* taking the transfer
        // queue lock: with thousands of MediaStore candidates this loop can
        // be slow, and holding an async write lock across it would starve
        // any concurrent reader of `transfer_queue()` (e.g. the Sync
        // Settings UI polling progress) for the whole duration.
        let pending_candidates = filter_pending_candidates(&self.index, &binding.id, &candidates)?;
        let scanned = candidates.len();
        let pending_n = pending_candidates.len();
        let ephemeral_selected: std::collections::HashSet<PathBuf> = pending_candidates
            .iter()
            .filter(|c| c.ephemeral)
            .map(|c| c.absolute_path.clone())
            .collect();
        // Ephemeral (cache-copy) candidates that were materialized but did not
        // survive filtering (e.g. content already unchanged) would otherwise
        // leak on disk forever — delete them now, before the lock, since we
        // don't need it for this cleanup.
        for c in &candidates {
            if c.ephemeral && !ephemeral_selected.contains(&c.absolute_path) {
                tokio::fs::remove_file(&c.absolute_path).await.ok();
            }
        }
        let pending: Vec<(LocalCandidate, String)> = {
            let mut queue = self.transfers.write().await;
            pending_candidates
                .into_iter()
                .map(|c| {
                    let tid = queue.enqueue_waiting(
                        &binding.id,
                        TransferDirection::Upload,
                        &c.relative_path,
                        Some(c.size),
                    );
                    (c, tid)
                })
                .collect()
        };

        let mut uploaded = 0usize;
        let mut failed = 0usize;
        let mut first_error: Option<String> = None;
        let mut pending_iter = pending.into_iter();
        // Files leave in waves (of `UPLOAD_PARALLELISM`, currently one). A failing
        // file is recorded with a retry deadline and the batch keeps going: it must
        // not abort the wave, the batch, or — via `sync_binding`'s `?` — the whole
        // tick, because that let a single unuploadable file at the head of the scan
        // order block every file behind it and stop downloads too, forever.
        loop {
            let wave: Vec<_> = pending_iter.by_ref().take(UPLOAD_PARALLELISM).collect();
            if wave.is_empty() {
                break;
            }

            let rels: Vec<String> = wave.iter().map(|(c, _)| c.relative_path.clone()).collect();
            let results = futures::future::join_all(
                wave.into_iter()
                    .map(|(candidate, waiting_id)| self.push_one(binding, candidate, waiting_id)),
            )
            .await;

            for (rel, result) in rels.into_iter().zip(results) {
                match result {
                    // Sent, or already up to date: either way this path is
                    // healthy, so any earlier failure recorded against it is
                    // stale and the ladder should restart from the bottom.
                    Ok(sent) => {
                        if sent {
                            uploaded += 1;
                        }
                        if let Err(e) = self.index.clear_upload_failure(&binding.id, &rel) {
                            warn!(binding = %binding.id, path = %rel, error = %e,
                                "clearing upload failure failed");
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        let msg = format!("{e:#}");
                        self.note_upload_failure(&binding.id, &rel, &msg);
                        if first_error.is_none() {
                            first_error = Some(msg);
                        }
                    }
                }
            }

            // Live progress for long Camera / folder uploads (Telegram is slow).
            if upload_only {
                let mut guard = self.statuses.write().await;
                if let Some(s) = guard.iter_mut().find(|s| s.binding_id == binding.id) {
                    s.uploading = uploaded;
                    s.failed = failed;
                    s.last_error = first_error.clone();
                }
            }
        }

        // Indexed files missing on disk are left untouched — no delete on the
        // server, and no automatic redownload either. Local absence (folder
        // wiped, file removed by hand) is not user intent to delete server
        // data, but it is not a download request either: the only way
        // content ever moves is an explicit user action (upload, or a
        // deliberate download elsewhere in the app), never something
        // inferred from local disk state.
        Ok(PushLocalResult {
            uploaded,
            scanned,
            pending: pending_n,
            failed,
            first_error,
        })
    }

    /// Records a failed upload and schedules its next attempt. Best-effort: if
    /// the index write itself fails the file simply gets retried next tick,
    /// which is the old behaviour and still forward progress for everything
    /// else in the batch.
    fn note_upload_failure(&self, binding_id: &str, relative_path: &str, error: &str) {
        let previous = self
            .index
            .get_upload_failure(binding_id, relative_path)
            .unwrap_or(None);
        let fail_count = previous.map_or(0, |f| f.fail_count) + 1;
        let next_attempt_ms = now_ms() + upload_backoff_ms(fail_count);
        warn!(
            binding = %binding_id,
            path = %relative_path,
            fail_count,
            error,
            "upload failed; deferring retry"
        );
        if let Err(e) = self.index.set_upload_failure(
            binding_id,
            relative_path,
            fail_count,
            next_attempt_ms,
            error,
        ) {
            warn!(binding = %binding_id, path = %relative_path, error = %e,
                "recording upload failure failed");
        }
    }

    /// Uploads a single candidate. `Ok(true)` means bytes reached the server,
    /// `Ok(false)` that there was nothing to send (content unchanged). On error only
    /// this candidate is cleaned up; the caller handles the rest of the batch.
    async fn push_one(
        &self,
        binding: &Binding,
        candidate: LocalCandidate,
        waiting_id: String,
    ) -> Result<bool> {
        {
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
                    &waiting_id,
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
                    if ephemeral {
                        tokio::fs::remove_file(&path).await.ok();
                    }
                    return Err(e).with_context(|| format!("hash {}", path.display()));
                }
            };

            if existing.as_ref().and_then(|e| e.content_hash.as_ref()) == Some(&hash) {
                // Content unchanged (e.g. mtime-only touch) — update index only.
                if let Some(mut e) = existing {
                    e.size = size;
                    e.mtime_ms = mtime;
                    self.index.upsert_entry(&binding.id, &e)?;
                }
                self.transfer_complete(&tid).await;
                if ephemeral {
                    tokio::fs::remove_file(&path).await.ok();
                }
                return Ok(false);
            }

            let (parent, filename) = split_parent_name(&rel);
            let remote_parent = join_remote(&binding.remote_root, &parent);
            if let Err(e) = self.ensure_remote_parents(binding, &parent).await {
                self.transfer_abandon(&tid).await;
                if ephemeral {
                    tokio::fs::remove_file(&path).await.ok();
                }
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
                if ephemeral {
                    tokio::fs::remove_file(&path).await.ok();
                }
                return Err(e).with_context(|| {
                    format!("upload {} → {}/{}", path.display(), remote_parent, filename)
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
        }

        Ok(true)
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
                            .transfer_begin(&binding.id, TransferDirection::Download, &rel, ev.size)
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
/// index (or have no index entry yet), minus any file still inside its retry
/// backoff window. Pure index reads — callers should not hold the transfer
/// queue lock while calling this so a slow/large scan cannot starve concurrent
/// readers of the queue (see `push_local`).
///
/// The backoff is time-based only. A user who does not want to wait out a
/// deadline has an explicit escape hatch: "Upload now" clears the whole
/// binding's backoff before ticking (see [`SyncEngine::retry_failed_uploads`]).
fn filter_pending_candidates(
    index: &LocalIndex,
    binding_id: &str,
    candidates: &[LocalCandidate],
) -> Result<Vec<LocalCandidate>> {
    let backoff = index.load_upload_backoff(binding_id)?;
    let now = now_ms();
    let mut out = Vec::new();
    for c in candidates {
        let existing = index.get_entry(binding_id, &c.relative_path)?;
        let needs = existing
            .as_ref()
            .is_none_or(|e| e.size != c.size || e.mtime_ms != c.mtime_ms);
        if !needs {
            continue;
        }
        if backoff
            .get(&c.relative_path)
            .is_some_and(|next| *next > now)
        {
            continue;
        }
        out.push(c.clone());
    }
    Ok(out)
}

/// Filters `candidates` down to those whose size/mtime differ from the
/// index (or have no index entry yet), enqueuing each as Waiting in
/// `queue`. The paired `String` is the waiting transfer id.
pub fn select_pending_uploads(
    index: &LocalIndex,
    binding_id: &str,
    candidates: &[LocalCandidate],
    queue: &mut TransferQueue,
) -> Result<Vec<(LocalCandidate, String)>> {
    let pending = filter_pending_candidates(index, binding_id, candidates)?;
    let mut out = Vec::with_capacity(pending.len());
    for c in pending {
        let tid = queue.enqueue_waiting(
            binding_id,
            TransferDirection::Upload,
            &c.relative_path,
            Some(c.size),
        );
        out.push((c, tid));
    }
    Ok(out)
}

fn strip_remote_root(path: &str, remote_root: &str) -> Option<String> {
    let path = path.trim_start_matches('/');
    // The result is joined onto the local sync root, so a remote path with a
    // relative segment (or a Windows drive / UNC prefix) would escape it.
    // Server-supplied paths are not trusted here.
    if path.split('/').any(|seg| seg == ".." || seg == ".")
        || path.contains('\\')
        || path.chars().nth(1) == Some(':')
    {
        return None;
    }
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
    fn strip_root_rejects_paths_that_escape_the_local_root() {
        assert_eq!(strip_remote_root("../../etc/passwd", ""), None);
        assert_eq!(strip_remote_root("docs/../../../etc/passwd", "docs"), None);
        assert_eq!(strip_remote_root("a/./b.txt", ""), None);
        assert_eq!(strip_remote_root(r"..\windows\system32", ""), None);
        assert_eq!(strip_remote_root("C:/windows/system32", ""), None);
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
            engine.statuses().await.iter().any(|s| s.binding_id == id),
            "status should exist after a successful tick"
        );

        engine.set_binding_enabled(&id, false).unwrap();
        assert!(
            engine.statuses().await.iter().all(|s| s.binding_id != id),
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
            engine.statuses().await.iter().all(|s| s.binding_id != id),
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

    /// Engine wired to port 9 ("discard"), which is never bound in test
    /// environments — every upload fails fast with connection-refused instead
    /// of a slow timeout (same trick used by api.rs's own tests).
    fn unreachable_engine(dir: &std::path::Path) -> SyncEngine {
        SyncEngine::open(
            SyncEngineConfig {
                poll_interval: Duration::from_secs(30),
                api: Arc::new(tokio::sync::RwLock::new(SarcaApi::new(
                    "http://127.0.0.1:9",
                    "",
                ))),
                data_dir: dir.to_path_buf(),
                media_source: Arc::new(crate::media_source::FsMediaSource),
            },
            Arc::new(KeepBothPrompt),
        )
        .unwrap()
    }

    fn cam_binding(local_path: &std::path::Path) -> Binding {
        Binding {
            id: "cam".into(),
            storage_id: uuid::Uuid::new_v4(),
            remote_root: "Camera".into(),
            local_path: local_path.to_string_lossy().into(),
            mode: BindingMode::AutoUpload,
            enabled: true,
        }
    }

    #[test]
    fn upload_backoff_climbs_then_saturates() {
        assert_eq!(upload_backoff_ms(1), 60_000);
        assert_eq!(upload_backoff_ms(2), 300_000);
        assert_eq!(upload_backoff_ms(5), 21_600_000);
        // Never grows past the last rung, and never returns 0 (which would be
        // no backoff at all — the bug this ladder exists to prevent).
        assert_eq!(upload_backoff_ms(99), 21_600_000);
        assert_eq!(upload_backoff_ms(0), 60_000);
    }

    #[tokio::test]
    async fn push_local_attempts_every_candidate_despite_failures() {
        // Regression test for "nothing syncs": an upload failure used to abort
        // the whole batch (and, via sync_binding's `?`, the whole tick), so one
        // unuploadable file at the head of the scan order starved every file
        // behind it forever. Now every candidate is attempted and the failures
        // are reported, not raised.
        const FILES: usize = 5;
        let dir = tempfile::tempdir().unwrap();
        let pics = dir.path().join("pics");
        std::fs::create_dir_all(&pics).unwrap();
        for i in 0..FILES {
            std::fs::write(pics.join(format!("{i}.jpg")), b"x").unwrap();
        }
        let engine = unreachable_engine(dir.path());
        let binding = cam_binding(&pics);

        let result = engine.push_local(&binding).await.expect(
            "per-file upload failures must not surface as a batch error, or the tick aborts and \
             downloads never run",
        );
        assert_eq!(result.failed, FILES, "every candidate must be attempted");
        assert_eq!(result.uploaded, 0);
        assert!(result.first_error.is_some(), "failures must be reported");
    }

    #[tokio::test]
    async fn failed_upload_is_deferred_then_retried_after_backoff() {
        // The point of the backoff: a file that just failed drops out of the
        // next scan's pending set, so it cannot re-occupy the head of the queue
        // on every 30s tick. Once its deadline passes it comes back.
        let dir = tempfile::tempdir().unwrap();
        let pics = dir.path().join("pics");
        std::fs::create_dir_all(&pics).unwrap();
        std::fs::write(pics.join("bad.jpg"), b"x").unwrap();
        let engine = unreachable_engine(dir.path());
        let binding = cam_binding(&pics);

        let first = engine.push_local(&binding).await.unwrap();
        assert_eq!(first.failed, 1);

        let recorded = engine
            .index
            .get_upload_failure(&binding.id, "bad.jpg")
            .unwrap()
            .expect("failure must be recorded");
        assert_eq!(recorded.fail_count, 1);
        assert!(recorded.next_attempt_ms > now_ms());

        // Second tick right away: still inside the window, so the file is not
        // even attempted and the failure count does not climb.
        let second = engine.push_local(&binding).await.unwrap();
        assert_eq!(second.pending, 0, "deferred file must not be re-attempted");
        assert_eq!(second.failed, 0);
        assert_eq!(
            engine
                .index
                .get_upload_failure(&binding.id, "bad.jpg")
                .unwrap()
                .unwrap()
                .fail_count,
            1,
            "a deferred file must not accrue failures it never attempted"
        );

        // Deadline in the past — back in the running, and the ladder advances.
        engine
            .index
            .set_upload_failure(&binding.id, "bad.jpg", 1, now_ms() - 1, "stale")
            .unwrap();
        let third = engine.push_local(&binding).await.unwrap();
        assert_eq!(third.failed, 1, "an expired deadline must retry the file");
        assert_eq!(
            engine
                .index
                .get_upload_failure(&binding.id, "bad.jpg")
                .unwrap()
                .unwrap()
                .fail_count,
            2
        );
    }

    #[tokio::test]
    async fn one_bad_file_does_not_block_the_rest_of_the_backlog() {
        // The head-of-line case, end to end: a file that is deferred must let
        // files discovered later through on the very next scan.
        let dir = tempfile::tempdir().unwrap();
        let pics = dir.path().join("pics");
        std::fs::create_dir_all(&pics).unwrap();
        std::fs::write(pics.join("0-bad.jpg"), b"x").unwrap();
        let engine = unreachable_engine(dir.path());
        let binding = cam_binding(&pics);

        engine.push_local(&binding).await.unwrap();

        // New photo arrives while the bad one is still deferred.
        std::fs::write(pics.join("1-new.jpg"), b"y").unwrap();
        let next = engine.push_local(&binding).await.unwrap();
        assert_eq!(
            next.pending, 1,
            "the new file must be picked up while the deferred one is skipped"
        );
        assert_eq!(next.failed, 1, "and it must actually be attempted");
    }

    #[tokio::test]
    async fn retry_failed_uploads_clears_the_backoff() {
        // "Upload now" escape hatch: the user cannot see the deadline, so an
        // explicit request must reconsider deferred files immediately.
        let dir = tempfile::tempdir().unwrap();
        let pics = dir.path().join("pics");
        std::fs::create_dir_all(&pics).unwrap();
        std::fs::write(pics.join("bad.jpg"), b"x").unwrap();
        let engine = unreachable_engine(dir.path());
        let binding = cam_binding(&pics);
        engine.index.upsert_binding(&binding).unwrap();

        engine.push_local(&binding).await.unwrap();
        assert_eq!(engine.push_local(&binding).await.unwrap().pending, 0);

        assert_eq!(engine.retry_failed_uploads(Some(&binding.id)).unwrap(), 1);
        assert_eq!(
            engine.push_local(&binding).await.unwrap().pending,
            1,
            "clearing the backoff must put the file back in the pending set"
        );
    }

    #[tokio::test]
    async fn successful_upload_clears_a_previous_failure() {
        // A file that failed once and then went through must not keep its
        // record, or its next failure would start partway up the ladder.
        let dir = tempfile::tempdir().unwrap();
        let pics = dir.path().join("pics");
        std::fs::create_dir_all(&pics).unwrap();
        let path = pics.join("photo.jpg");
        std::fs::write(&path, b"x").unwrap();
        let engine = unreachable_engine(dir.path());
        let binding = cam_binding(&pics);

        engine.push_local(&binding).await.unwrap();
        assert!(engine
            .index
            .get_upload_failure(&binding.id, "photo.jpg")
            .unwrap()
            .is_some());

        // Pre-seed the index with the file's current content hash so push_one
        // takes the "content unchanged" path — a success that sends no bytes,
        // which must still count as the file being healthy.
        let hash = crate::hash::sha256_file(&path).await.unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let mtime = mtime_ms_from_system(meta.modified().unwrap());
        engine
            .index
            .upsert_entry(
                &binding.id,
                &IndexEntry {
                    relative_path: "photo.jpg".into(),
                    size: 0, // differs, so the file is still a candidate
                    mtime_ms: mtime,
                    content_hash: Some(hash),
                    remote_file_id: None,
                    last_cursor: 0,
                },
            )
            .unwrap();
        engine.retry_failed_uploads(Some(&binding.id)).unwrap();

        let result = engine.push_local(&binding).await.unwrap();
        assert_eq!(result.failed, 0);
        assert!(
            engine
                .index
                .get_upload_failure(&binding.id, "photo.jpg")
                .unwrap()
                .is_none(),
            "a healthy file must not keep a stale failure record"
        );
    }

    struct FixedMediaSource(Vec<LocalCandidate>);

    #[async_trait]
    impl LocalMediaSource for FixedMediaSource {
        async fn list_candidates(&self, _binding: &Binding) -> Result<Vec<LocalCandidate>> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn push_local_deletes_ephemeral_files_not_selected_for_upload() {
        // Regression test for lazy-materialize cleanup: a candidate whose
        // ephemeral (cache-copy) file was already materialized, but which
        // doesn't need uploading (content unchanged), must not leak its
        // cache file on disk forever.
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let unchanged_path = cache.join("unchanged.jpg");
        std::fs::write(&unchanged_path, b"x").unwrap();

        let engine = SyncEngine::open(
            SyncEngineConfig {
                poll_interval: Duration::from_secs(30),
                api: Arc::new(tokio::sync::RwLock::new(SarcaApi::new(
                    "http://127.0.0.1:9",
                    "",
                ))),
                data_dir: dir.path().to_path_buf(),
                media_source: Arc::new(FixedMediaSource(vec![LocalCandidate {
                    relative_path: "unchanged.jpg".into(),
                    absolute_path: unchanged_path.clone(),
                    size: 1,
                    mtime_ms: 5,
                    ephemeral: true,
                }])),
            },
            Arc::new(KeepBothPrompt),
        )
        .unwrap();
        let binding = Binding {
            id: "cam".into(),
            storage_id: uuid::Uuid::new_v4(),
            remote_root: "Camera".into(),
            local_path: cache.to_string_lossy().into(),
            mode: BindingMode::AutoUpload,
            enabled: true,
        };
        // Pre-seed the index so this candidate looks already-synced (unchanged).
        engine
            .index
            .upsert_entry(
                &binding.id,
                &IndexEntry {
                    relative_path: "unchanged.jpg".into(),
                    size: 1,
                    mtime_ms: 5,
                    content_hash: Some("whatever".into()),
                    remote_file_id: None,
                    last_cursor: 0,
                },
            )
            .unwrap();

        let uploaded = engine.push_local(&binding).await.unwrap();
        assert_eq!(uploaded.uploaded, 0);
        assert_eq!(uploaded.scanned, 1);
        assert_eq!(uploaded.pending, 0);
        assert!(
            !unchanged_path.exists(),
            "ephemeral file not selected for upload must be cleaned up"
        );
    }

    #[tokio::test]
    async fn push_local_never_drops_index_entry_for_missing_local_file() {
        // Regression test: a file that was uploaded/synced and then removed
        // locally (folder wiped by hand, disk cleanup, etc.) must never be
        // deleted on the server as a side effect, and must not trigger an
        // automatic redownload either — nothing moves except on an explicit
        // user action. push_local must leave the index entry untouched and
        // must not touch the network at all (asserted here by using an
        // unreachable API and expecting success: if push_local attempted a
        // download it would fail against port 9).
        let dir = tempfile::tempdir().unwrap();
        let engine = SyncEngine::open(
            SyncEngineConfig {
                poll_interval: Duration::from_secs(30),
                api: Arc::new(tokio::sync::RwLock::new(SarcaApi::new(
                    "http://127.0.0.1:9",
                    "",
                ))),
                data_dir: dir.path().to_path_buf(),
                media_source: Arc::new(FixedMediaSource(vec![])),
            },
            Arc::new(KeepBothPrompt),
        )
        .unwrap();
        let binding = Binding {
            id: "cam".into(),
            storage_id: uuid::Uuid::new_v4(),
            remote_root: "Camera".into(),
            local_path: dir.path().join("pics").to_string_lossy().into(),
            mode: BindingMode::AutoUpload,
            enabled: true,
        };
        engine
            .index
            .upsert_entry(
                &binding.id,
                &IndexEntry {
                    relative_path: "gone.jpg".into(),
                    size: 1,
                    mtime_ms: 5,
                    content_hash: Some("whatever".into()),
                    remote_file_id: None,
                    last_cursor: 0,
                },
            )
            .unwrap();

        let result = engine.push_local(&binding).await;
        assert!(
            result.is_ok(),
            "push_local must not touch the network for a missing local file"
        );

        assert!(
            engine
                .index
                .get_entry(&binding.id, "gone.jpg")
                .unwrap()
                .is_some(),
            "missing local file must never drop the index entry or imply a remote delete"
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
        assert!(!pending[0].1.is_empty());
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

    fn sync_binding_fixture(dir: &std::path::Path) -> Binding {
        Binding {
            id: "sync1".into(),
            storage_id: uuid::Uuid::new_v4(),
            remote_root: "Root".into(),
            local_path: dir.join("sync1").to_string_lossy().into(),
            mode: BindingMode::Sync,
            enabled: true,
        }
    }

    #[tokio::test]
    async fn remote_has_changes_false_for_empty_changelog() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let _ = sock.read(&mut buf).await;
            let body = br#"{"events":[],"next_cursor":5,"has_more":false}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.write_all(body).await;
        });

        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path());
        engine
            .set_credentials(format!("http://{addr}"), "t".into())
            .await;
        let binding = sync_binding_fixture(dir.path());
        engine.upsert_binding(&binding).unwrap();
        // A cursor of 0 short-circuits to `true` (never bootstrapped), so
        // this must be set to something else to actually exercise the
        // changelog request.
        engine.index.set_cursor(&binding.id, 5).unwrap();

        assert!(!engine.remote_has_changes().await.unwrap());
    }

    #[tokio::test]
    async fn remote_has_changes_true_for_nonempty_changelog() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let _ = sock.read(&mut buf).await;
            let body = format!(
                r#"{{"events":[{{"id":1,"storage_id":"{}","file_id":null,"path":"a.jpg","op":"upsert","size":3,"is_file":true,"content_hash":null,"source_mtime":null,"created_at":"2024-01-01T00:00:00Z"}}],"next_cursor":6,"has_more":false}}"#,
                uuid::Uuid::nil()
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.write_all(body.as_bytes()).await;
        });

        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path());
        engine
            .set_credentials(format!("http://{addr}"), "t".into())
            .await;
        let binding = sync_binding_fixture(dir.path());
        engine.upsert_binding(&binding).unwrap();
        engine.index.set_cursor(&binding.id, 5).unwrap();

        assert!(engine.remote_has_changes().await.unwrap());
    }
}
