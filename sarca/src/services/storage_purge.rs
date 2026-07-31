use std::time::Duration;

use sqlx::{Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::{
    common::telegram_api::bot_api::TelegramBotApi,
    errors::SarcaResult,
    repositories::{
        chunk_replicas::ChunkReplicasRepository,
        files::FilesRepository,
        storage_purge::StoragePurgeRepository,
        storage_workers::StorageWorkersRepository,
    },
    services::storage_workers_scheduler::StorageWorkersScheduler,
};

const PURGE_BATCH_SIZE: i64 = 10;
const MAX_ATTEMPTS: i32 = 8;
const IN_PROGRESS_LEASE: Duration = Duration::from_mins(10);
const FAILED_BATCH_DELAY: Duration = Duration::from_secs(1);

pub struct StoragePurgeService;

impl StoragePurgeService {
    pub fn spawn_loop(db: SqlitePool, base_url: String, rate_limit: u8, idle: Duration) {
        tokio::spawn(async move {
            loop {
                Self::run_once(&db, &base_url, rate_limit, idle).await;
            }
        });
    }

    async fn run_once(db: &SqlitePool, base_url: &str, rate_limit: u8, idle: Duration) {
        let repo = StoragePurgeRepository::new(db);
        if let Err(e) = repo.requeue_stale_in_progress(IN_PROGRESS_LEASE).await {
            tracing::warn!("[STORAGE PURGE] failed to requeue stale in-progress messages: {e}");
        }
        let batch = match repo.claim_pending(PURGE_BATCH_SIZE).await {
            Ok(batch) => batch,
            Err(e) => {
                tracing::warn!("[STORAGE PURGE] failed to claim pending messages: {e}");
                tokio::time::sleep(idle).await;
                return;
            },
        };

        if batch.is_empty() {
            if let Err(e) = repo.try_complete_jobs().await {
                tracing::warn!("[STORAGE PURGE] failed to complete drained jobs: {e}");
            }
            tokio::time::sleep(idle).await;
            return;
        }

        let mut had_telegram_error = false;
        for row in batch {
            let api = TelegramBotApi::new(base_url, StorageWorkersScheduler::new(db, rate_limit));
            match api.delete_message_with_token(row.chat_id, row.message_id, &row.bot_token).await {
                Ok(()) => {
                    if let Err(e) = repo.mark_done(row.id).await {
                        tracing::warn!(
                            "[STORAGE PURGE] failed to mark message {} done: {e}",
                            row.id
                        );
                    }
                },
                Err(e) => {
                    had_telegram_error = true;
                    tracing::warn!(
                        "[STORAGE PURGE] job {} message {} chat {} mid {}: {e}",
                        row.job_id,
                        row.id,
                        row.chat_id,
                        row.message_id
                    );
                    let result = if is_terminal_attempt(row.attempts) {
                        repo.mark_failed(row.id, &e.to_string()).await
                    } else {
                        repo.mark_retry(row.id, &e.to_string()).await
                    };
                    if let Err(mark_error) = result {
                        tracing::warn!(
                            "[STORAGE PURGE] failed to update message {} after Telegram error: {mark_error}",
                            row.id
                        );
                    }
                },
            }
        }

        if let Err(e) = repo.try_complete_jobs().await {
            tracing::warn!("[STORAGE PURGE] failed to complete drained jobs: {e}");
        }
        if had_telegram_error {
            tokio::time::sleep(FAILED_BATCH_DELAY).await;
        }
    }
}

/// Snapshot this storage's Telegram messages and token before deleting its rows.
/// A missing bot is logged but deliberately does not prevent deletion.
pub async fn snapshot_storage_telegram_purge(
    db: &SqlitePool,
    storage_id: Uuid,
) -> SarcaResult<Option<(String, Vec<(i64, i64)>)>> {
    let mut messages =
        ChunkReplicasRepository::new(db).list_telegram_messages_for_storage(storage_id).await?;
    messages.extend(FilesRepository::new(db).list_derived_messages_for_storage(storage_id).await?);
    messages.sort_unstable();
    messages.dedup();

    if messages.is_empty() {
        return Ok(None);
    }

    let Some(worker) = StorageWorkersRepository::new(db).get_by_storage_id(storage_id).await?
    else {
        tracing::error!(
            "[STORAGE PURGE] storage {storage_id} has {} Telegram message(s) but no bot token; skipping Telegram GC",
            messages.len()
        );
        return Ok(None);
    };

    Ok(Some((worker.token, messages)))
}

/// Enqueue a previously captured Telegram snapshot in the caller's transaction.
pub async fn enqueue_storage_telegram_purge_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    storage_id: Uuid,
    snapshot: Option<(String, Vec<(i64, i64)>)>,
) -> SarcaResult<()> {
    let Some((token, messages)) = snapshot else {
        return Ok(());
    };
    StoragePurgeRepository::enqueue_in_tx(tx, Uuid::new_v4(), storage_id, &token, &messages).await
}

const fn is_terminal_attempt(attempts: i32) -> bool {
    attempts >= MAX_ATTEMPTS
}

#[cfg(test)]
mod tests {
    use super::is_terminal_attempt;

    #[test]
    fn eighth_attempt_is_terminal() {
        assert!(is_terminal_attempt(8));
        assert!(is_terminal_attempt(9));
        assert!(!is_terminal_attempt(7));
    }
}
