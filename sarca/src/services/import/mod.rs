//! Server-side import from another cloud.
//!
//! The copy runs here, not on a client: the phone or laptop that set it up can
//! close, and the files go from the source straight into the same upload
//! pipeline a client upload ends in — same spool, same spool budget, same
//! resume-by-content-hash after an interruption.
//!
//! A job lists the source once (`scanning`), recording every file as an
//! `import_items` row, then copies the rows one at a time (`running`). Each copy
//! is handed off as soon as it is spooled, exactly as a client hands off, so the
//! next download overlaps the previous file's relay to Telegram; the spool
//! budget is what keeps that from running ahead of the uplink.
//!
//! v1 is superuser-only. An import makes this server fetch any URL it is given,
//! and with credentials it holds, which is not a lever for every account.

pub mod webdav;

use std::{collections::HashSet, path::Path, sync::Arc, time::Duration};

use sha2::{Digest, Sha256};
use tokio::{io::AsyncWriteExt, sync::mpsc};
use uuid::Uuid;

use self::webdav::{WebDavClient, WebDavError, parse_root_url};
use crate::{
    common::{
        access::check_access,
        jwt_manager::AuthUser,
        routing::app_state::AppState,
        supervisor::spawn_supervised,
    },
    errors::{SarcaError, SarcaResult},
    models::{
        access::AccessType,
        files::InFile,
        imports::{
            ITEM_DONE,
            ITEM_FAILED,
            ITEM_PENDING,
            ITEM_SKIPPED,
            ImportItem,
            ImportJob,
            ImportJobSummary,
            JOB_CANCELLED,
            JOB_DONE,
            JOB_FAILED,
            JOB_PAUSED,
            JOB_RUNNING,
            JOB_SCANNING,
        },
    },
    repositories::{
        access::AccessRepository,
        files::FilesRepository,
        imports::{ImportsRepository, NewImportItem, NewImportJob},
        storage_workers::StorageWorkersRepository,
        users::UsersRepository,
    },
    services::files::{FilesService, SpoolAdmission, available_bytes, spool_usage},
};

pub const SOURCE_WEBDAV: &str = "webdav";

/// Pause between runner ticks when there is nothing to start.
const IDLE: Duration = Duration::from_secs(5);
/// How long to wait before asking the spool again after it said "busy".
const SPOOL_WAIT: Duration = Duration::from_secs(5);
/// Tries per file before it is left as failed. A failed file does not stop the
/// job; "retry failed" puts it back.
const MAX_ATTEMPTS: i64 = 3;

/// Message stored on a job whose password is gone — a restored backup never
/// carries one.
const NO_SECRET: &str = "the password is not stored on this server (it was restored from a \
                         backup) — enter it again to continue";

pub struct ImportRunner;

impl ImportRunner {
    pub fn spawn(state: Arc<AppState>) {
        spawn_supervised("import", move || {
            let state = state.clone();
            async move {
                match ImportsRepository::new(&state.db).requeue_in_progress().await {
                    Ok(0) => {},
                    Ok(n) => tracing::info!("[IMPORT] {n} interrupted file(s) back in the queue"),
                    Err(e) => tracing::warn!("[IMPORT] could not requeue interrupted files: {e}"),
                }
                loop {
                    let delay = Self::run_once(&state).await;
                    tokio::time::sleep(delay).await;
                }
            }
        });
    }

    /// One step of the oldest active job; returns how long to wait before the next.
    async fn run_once(state: &Arc<AppState>) -> Duration {
        let repo = ImportsRepository::new(&state.db);
        let job = match repo.next_active_job().await {
            Ok(Some(job)) => job,
            Ok(None) => return IDLE,
            Err(e) => {
                tracing::warn!("[IMPORT] could not read jobs: {e}");
                return IDLE;
            },
        };

        let (client, user) = match Self::prepare(state, &job).await {
            Ok(ready) => ready,
            Err(message) => {
                tracing::warn!("[IMPORT] job {} stopped: {message}", job.id);
                let _ = repo
                    .transition(job.id, &[JOB_SCANNING, JOB_RUNNING], JOB_FAILED, Some(&message))
                    .await;
                return Duration::ZERO;
            },
        };

        if job.status == JOB_SCANNING {
            match Self::scan(state, &job, &client).await {
                Ok(true) => {
                    let _ = repo.finish_scan(job.id).await;
                },
                // Paused or cancelled mid-listing.
                Ok(false) => {},
                Err(e) => {
                    tracing::warn!("[IMPORT] listing for job {} failed: {e}", job.id);
                    let _ = repo
                        .transition(job.id, &[JOB_SCANNING], JOB_FAILED, Some(&e.to_string()))
                        .await;
                },
            }
            return Duration::ZERO;
        }

        let item = match repo.claim_next_item(job.id).await {
            Ok(Some(item)) => item,
            Ok(None) => {
                // Nothing left to start. Done once the last relay has reported.
                if repo.is_drained(job.id).await.unwrap_or(false) {
                    let failed = repo.count_failed(job.id).await.unwrap_or(0);
                    let note =
                        (failed > 0).then(|| format!("{failed} file(s) could not be copied"));
                    let _ =
                        repo.transition(job.id, &[JOB_RUNNING], JOB_DONE, note.as_deref()).await;
                    tracing::info!("[IMPORT] job {} finished", job.id);
                    return Duration::ZERO;
                }
                return IDLE;
            },
            Err(e) => {
                tracing::warn!("[IMPORT] could not claim the next file: {e}");
                return IDLE;
            },
        };

        Self::copy_item(state, &job, &client, &user, item).await;
        Duration::ZERO
    }

    /// The source client and the account the copy runs as, or why the job
    /// cannot go on at all.
    async fn prepare(
        state: &AppState,
        job: &ImportJob,
    ) -> Result<(WebDavClient, AuthUser), String> {
        let Some(secret) = job.secret.clone() else {
            return Err(NO_SECRET.to_owned());
        };
        let user = UsersRepository::new(&state.db)
            .get_by_id(job.user_id)
            .await
            .map_err(|_| "the account that started this import no longer exists".to_owned())?;
        if user.disabled_at.is_some() {
            return Err("the account that started this import is disabled".to_owned());
        }
        let root = parse_root_url(&job.source_url).map_err(|e| e.to_string())?;
        let client =
            WebDavClient::new(root, job.username.clone(), secret).map_err(|e| e.to_string())?;
        Ok((client, AuthUser::new(user.id, user.email)))
    }

    /// List the whole source into `import_items`, creating its folders in the
    /// target as they are found. `Ok(false)`: the job left `scanning` meanwhile.
    async fn scan(
        state: &AppState,
        job: &ImportJob,
        client: &WebDavClient,
    ) -> Result<bool, WebDavError> {
        let repo = ImportsRepository::new(&state.db);
        let files = FilesRepository::new(&state.db);
        let mut created = HashSet::new();
        ensure_folder(&files, job.storage_id, &job.target_path, &mut created).await;

        let mut queue = vec![String::new()];
        let mut found = 0usize;
        while let Some(dir) = queue.pop() {
            match repo.get_job(job.id).await {
                Ok(Some(current)) if current.status == JOB_SCANNING => {},
                _ => return Ok(false),
            }
            let entries = client.list(&dir).await?;
            let mut batch = Vec::new();
            for entry in entries {
                if entry.is_dir {
                    if let Some(target) = join_target(&job.target_path, &entry.rel_path) {
                        ensure_folder(&files, job.storage_id, &target, &mut created).await;
                    }
                    queue.push(entry.rel_path);
                } else {
                    batch.push(NewImportItem {
                        rel_path: entry.rel_path,
                        size: i64::try_from(entry.size).unwrap_or(i64::MAX),
                        source_mtime: entry.mtime,
                    });
                }
            }
            found += batch.len();
            repo.insert_items(job.id, &batch)
                .await
                .map_err(|e| WebDavError(format!("could not record the listing: {e}")))?;
        }
        tracing::info!("[IMPORT] job {} listed {found} file(s)", job.id);
        Ok(true)
    }

    /// Copy one file: download it into the upload spool and hand it to the
    /// upload pipeline. Returns once the pipeline has taken the spool; the item
    /// is settled by the relay task when Telegram is done with it.
    async fn copy_item(
        state: &Arc<AppState>,
        job: &ImportJob,
        client: &WebDavClient,
        user: &AuthUser,
        item: ImportItem,
    ) {
        let repo = ImportsRepository::new(&state.db);
        let Some(path) = join_target(&job.target_path, &item.rel_path) else {
            let _ = repo.finish_item(item.id, ITEM_FAILED, Some("invalid file name")).await;
            return;
        };

        // Already here from an earlier run of this import, or by hand. Size is
        // the only cheap comparison: the source's hash would need the download
        // this check exists to avoid.
        if let Ok(existing) =
            FilesRepository::new(&state.db).get_file_by_path(&path, job.storage_id).await
        {
            if existing.is_uploaded && existing.size == item.size {
                let _ = repo.finish_item(item.id, ITEM_SKIPPED, None).await;
                return;
            }
        }

        let size = item.size.max(0).cast_unsigned();
        let budget = state.config.upload_spool_budget_bytes();
        loop {
            let available = available_bytes(Path::new(&state.config.work_dir));
            match spool_usage().admit(size, budget, available) {
                SpoolAdmission::Admit => break,
                SpoolAdmission::Busy => {
                    tokio::time::sleep(SPOOL_WAIT).await;
                    if !matches!(repo.get_job(job.id).await, Ok(Some(j)) if j.status == JOB_RUNNING)
                    {
                        let _ = repo.release_item(item.id).await;
                        return;
                    }
                },
                SpoolAdmission::NoRoom => {
                    let _ = repo
                        .finish_item(
                            item.id,
                            ITEM_FAILED,
                            Some("not enough free space under WORK_DIR for this file"),
                        )
                        .await;
                    return;
                },
            }
        }

        let spool = Path::new(&state.config.work_dir)
            .join("uploads")
            .join(format!("{}.upload", Uuid::new_v4()));
        let (file_size, hash) = match download(client, &item.rel_path, &spool).await {
            Ok(done) => done,
            Err(e) => {
                let _ = tokio::fs::remove_file(&spool).await;
                tracing::warn!("[IMPORT] download of an item in job {} failed: {e}", job.id);
                if e.is_auth() {
                    let _ = repo.release_item(item.id).await;
                    let _ = repo
                        .transition(job.id, &[JOB_RUNNING], JOB_FAILED, Some(&e.to_string()))
                        .await;
                } else {
                    Self::settle_failure(&repo, &item, &e.to_string()).await;
                }
                return;
            },
        };

        let chunk_size = i64::try_from(state.config.default_chunk_size_bytes()).unwrap_or(i64::MAX);
        let in_file = InFile::new(path, file_size, job.storage_id)
            .with_chunk_size(chunk_size)
            .with_source_times(item.source_mtime, item.source_mtime)
            .with_content_hash(Some(hash));

        let (progress_tx, mut progress_rx) = mpsc::channel(64);
        let task_state = state.clone();
        let task_user = user.clone();
        tokio::spawn(async move {
            let state = task_state;
            let result = FilesService::new(
                &state.db,
                state.tx.clone(),
                &state.config.telegram_api_base_url,
                state.config.telegram_rate_limit,
                &state.config.work_dir,
            )
            .upload_anyway_from_path_with_progress(
                in_file,
                spool.clone(),
                file_size,
                &task_user,
                Some(progress_tx),
                None,
                false,
            )
            .await;
            let repo = ImportsRepository::new(&state.db);
            match result {
                Ok(()) => {
                    let _ = repo.finish_item(item.id, ITEM_DONE, None).await;
                },
                Err(e) => {
                    let _ = tokio::fs::remove_file(&spool).await;
                    Self::settle_failure(&repo, &item, &e.to_string()).await;
                },
            }
        });

        // `spooled` is the hand-off; a closed channel means the upload ended
        // before that (an error, or a duplicate settled at once).
        while let Some(event) = progress_rx.recv().await {
            if event.phase == "spooled" {
                break;
            }
        }
    }

    async fn settle_failure(repo: &ImportsRepository<'_>, item: &ImportItem, error: &str) {
        let status = if item.attempts >= MAX_ATTEMPTS { ITEM_FAILED } else { ITEM_PENDING };
        let _ = repo.finish_item(item.id, status, Some(error)).await;
    }
}

/// Stream `rel_path` into `spool`, hashing as it arrives. Returns the size and
/// the `sha256:` content hash the upload pipeline resumes by.
async fn download(
    client: &WebDavClient,
    rel_path: &str,
    spool: &Path,
) -> Result<(i64, String), WebDavError> {
    let io_error = |e: std::io::Error| WebDavError(format!("could not write the spool file: {e}"));
    if let Some(dir) = spool.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io_error)?;
    }
    let mut response = client.get(rel_path).await?;
    let mut file = tokio::fs::File::create(spool).await.map_err(io_error)?;
    let mut hasher = Sha256::new();
    let mut size = 0i64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| WebDavError(format!("the download broke off: {}", webdav::describe(&e))))?
    {
        hasher.update(&chunk);
        size = size.saturating_add(chunk.len() as i64);
        file.write_all(&chunk).await.map_err(io_error)?;
    }
    file.flush().await.map_err(io_error)?;
    let hash = hasher.finalize().iter().fold(String::from("sha256:"), |mut acc, byte| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{byte:02x}");
        acc
    });
    Ok((size, hash))
}

/// Create `path` and every folder above it, once per scan.
async fn ensure_folder(
    files: &FilesRepository<'_>,
    storage_id: Uuid,
    path: &str,
    created: &mut HashSet<String>,
) {
    let mut prefix = String::new();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        prefix.push_str(segment);
        prefix.push('/');
        if created.insert(prefix.clone()) {
            // An existing folder answers "already exists", which is the point.
            let _ = files.create_folder(InFile::new(prefix.clone(), 0, storage_id)).await;
        }
    }
}

/// `target/rel` as a storage path, or `None` when either holds a segment a
/// storage path may not (`..`, `.`).
pub fn join_target(target: &str, rel: &str) -> Option<String> {
    let mut parts = Vec::new();
    for segment in target.split(['/', '\\']).chain(rel.split('/')) {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if segment == "." || segment == ".." {
            return None;
        }
        parts.push(segment);
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Normalise the folder an import lands in: `/`-separated, no empty, `.` or
/// `..` segments, no leading or trailing slash. `""` is the storage root.
pub fn normalize_target(raw: &str) -> SarcaResult<String> {
    let mut parts = Vec::new();
    for segment in raw.split(['/', '\\']) {
        let segment = segment.trim();
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(SarcaError::InvalidPath);
        }
        parts.push(segment);
    }
    Ok(parts.join("/"))
}

/// What the API does with jobs. Every method expects the caller to be the
/// superuser; the router checks that before getting here.
pub struct ImportService<'d> {
    state: &'d AppState,
}

impl<'d> ImportService<'d> {
    pub fn new(state: &'d AppState) -> Self {
        Self {
            state,
        }
    }

    fn repo(&self) -> ImportsRepository<'d> {
        ImportsRepository::new(&self.state.db)
    }

    pub async fn list(&self) -> SarcaResult<Vec<ImportJobSummary>> {
        self.repo().list_summaries().await
    }

    pub async fn get(&self, id: Uuid) -> SarcaResult<(ImportJobSummary, Vec<ImportItem>)> {
        let summary = self.repo().get_summary(id).await?;
        let failed = self.repo().list_failed(id, 100).await?;
        Ok((summary, failed))
    }

    /// Check the source answers with these credentials, then queue the job.
    /// The check is the one moment a mistyped address or password can be told
    /// to the person typing it, rather than found later as a failed job.
    pub async fn create(&self, user: &AuthUser, input: NewImport) -> SarcaResult<ImportJobSummary> {
        check_access(
            &AccessRepository::new(&self.state.db),
            user.id,
            input.storage_id,
            &AccessType::W,
        )
        .await?;
        if !StorageWorkersRepository::new(&self.state.db).storage_has_any(input.storage_id).await? {
            return Err(SarcaError::StorageDoesNotHaveWorkers);
        }
        let target_path = normalize_target(&input.target_path)?;
        let root = parse_root_url(&input.url).map_err(|e| SarcaError::ImportSource(e.0))?;
        let username = input.username.trim().to_owned();
        let client = WebDavClient::new(root.clone(), username.clone(), input.password.clone())
            .map_err(|e| SarcaError::ImportSource(e.0))?;
        client.list("").await.map_err(|e| SarcaError::ImportSource(e.0))?;

        let id = self
            .repo()
            .create_job(NewImportJob {
                user_id: user.id,
                storage_id: input.storage_id,
                source_kind: SOURCE_WEBDAV,
                source_url: root.as_str(),
                username: &username,
                secret: &input.password,
                target_path: &target_path,
            })
            .await?;
        tracing::info!("[IMPORT] job {id} created");
        self.repo().get_summary(id).await
    }

    pub async fn pause(&self, id: Uuid) -> SarcaResult<ImportJobSummary> {
        self.move_job(id, &[JOB_SCANNING, JOB_RUNNING], JOB_PAUSED).await
    }

    /// Continue a paused or failed job. `password` replaces the stored one —
    /// how a job restored from a backup, or one whose password changed at the
    /// source, gets going again.
    pub async fn resume(&self, id: Uuid, password: Option<&str>) -> SarcaResult<ImportJobSummary> {
        let job = self.job(id).await?;
        if let Some(password) = password.filter(|p| !p.is_empty()) {
            self.repo().set_secret(id, password).await?;
        } else if job.secret.is_none() {
            return Err(SarcaError::ImportSource(NO_SECRET.to_owned()));
        }
        let to = if job.scanned { JOB_RUNNING } else { JOB_SCANNING };
        self.move_job(id, &[JOB_PAUSED, JOB_FAILED], to).await
    }

    /// Put the files that failed back in the queue and run the job again.
    pub async fn retry_failed(&self, id: Uuid) -> SarcaResult<ImportJobSummary> {
        let job = self.job(id).await?;
        self.repo().retry_failed(id).await?;
        if job.status == JOB_DONE || job.status == JOB_FAILED {
            let to = if job.scanned { JOB_RUNNING } else { JOB_SCANNING };
            self.repo().transition(id, &[JOB_DONE, JOB_FAILED], to, None).await?;
        }
        self.repo().get_summary(id).await
    }

    pub async fn cancel(&self, id: Uuid) -> SarcaResult<ImportJobSummary> {
        self.move_job(id, &[JOB_SCANNING, JOB_RUNNING, JOB_PAUSED], JOB_CANCELLED).await
    }

    /// Forget a job. Files it already copied stay in the storage.
    pub async fn delete(&self, id: Uuid) -> SarcaResult<()> {
        let job = self.job(id).await?;
        if job.status == JOB_SCANNING || job.status == JOB_RUNNING {
            return Err(SarcaError::ImportState(
                "pause or cancel the import before removing it".to_owned(),
            ));
        }
        self.repo().delete_job(id).await?;
        Ok(())
    }

    async fn job(&self, id: Uuid) -> SarcaResult<ImportJob> {
        self.repo().get_job(id).await?.ok_or_else(|| SarcaError::DoesNotExist("import".to_owned()))
    }

    async fn move_job(&self, id: Uuid, from: &[&str], to: &str) -> SarcaResult<ImportJobSummary> {
        let job = self.job(id).await?;
        if !self.repo().transition(id, from, to, None).await? {
            return Err(SarcaError::ImportState(format!(
                "the import is {} and cannot be {}",
                job.status,
                match to {
                    JOB_PAUSED => "paused",
                    JOB_CANCELLED => "cancelled",
                    _ => "resumed",
                }
            )));
        }
        self.repo().get_summary(id).await
    }
}

/// What the API hands [`ImportService::create`].
#[derive(Debug, Clone)]
pub struct NewImport {
    pub storage_id: Uuid,
    pub url: String,
    pub username: String,
    pub password: String,
    pub target_path: String,
}

#[cfg(test)]
mod tests {
    use super::{join_target, normalize_target};

    #[test]
    fn joins_target_and_relative_path() {
        assert_eq!(join_target("", "a/b.jpg").as_deref(), Some("a/b.jpg"));
        assert_eq!(
            join_target("Imports/Nextcloud", "a/b.jpg").as_deref(),
            Some("Imports/Nextcloud/a/b.jpg")
        );
        assert_eq!(join_target("x", "../etc/passwd"), None);
        assert_eq!(join_target("", ""), None);
    }

    #[test]
    fn normalizes_target_folder() {
        assert_eq!(normalize_target(" /Imports//Cloud/ ").unwrap(), "Imports/Cloud");
        assert_eq!(normalize_target("").unwrap(), "");
        assert!(normalize_target("a/../b").is_err());
    }
}
