use std::time::Duration;

use sqlx::PgPool;

use crate::{
    common::telegram_api::bot_api::{is_chat_dead_error, TelegramBotApi},
    repositories::{chunk_replicas::ChunkReplicasRepository, storage_channels::StorageChannelsRepository},
};

use super::storage_workers_scheduler::StorageWorkersScheduler;

const BATCH_SIZE: i64 = 25;

pub struct ReplicationService;

impl ReplicationService {
    /// Process one batch of pending/failed replicas: for each, find a live source replica
    /// for that chunk (preferring `copyMessage`, falling back to download + re-upload) and
    /// push it to the job's target channel. Returns how many jobs were attempted.
    pub async fn run_once(db: &PgPool, base_url: &str, rate_limit: u8) -> usize {
        let replicas_repo = ChunkReplicasRepository::new(db);
        let channels_repo = StorageChannelsRepository::new(db);

        let jobs = match replicas_repo.list_pending(BATCH_SIZE).await {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::warn!("[REPLICATION] failed to list pending jobs: {e}");
                return 0;
            }
        };

        let count = jobs.len();

        for job in jobs {
            let scheduler = StorageWorkersScheduler::new(db, rate_limit);
            let api = TelegramBotApi::new(base_url, scheduler);

            let source = match replicas_repo
                .find_source_for_chunk(job.chunk_id, job.channel_id)
                .await
            {
                Ok(Some(source)) => source,
                Ok(None) => {
                    tracing::debug!(
                        "[REPLICATION] no live source replica yet for chunk {}; will retry later",
                        job.chunk_id
                    );
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        "[REPLICATION] failed to find source replica for chunk {}: {e}",
                        job.chunk_id
                    );
                    continue;
                }
            };

            let result = match (source.telegram_file_id.clone(), source.telegram_message_id) {
                (Some(file_id), Some(message_id)) => {
                    api.copy_message(
                        source.chat_id,
                        message_id,
                        job.target_chat_id,
                        &file_id,
                        source.storage_id,
                    )
                    .await
                }
                (Some(file_id), None) => match api.download(&file_id, source.storage_id).await {
                    Ok(bytes) => api.upload(&bytes, job.target_chat_id, source.storage_id).await,
                    Err(e) => Err(e),
                },
                (None, _) => {
                    tracing::warn!(
                        "[REPLICATION] source replica for chunk {} has no telegram_file_id",
                        job.chunk_id
                    );
                    continue;
                }
            };

            match result {
                Ok(outcome) => {
                    if let Err(e) = replicas_repo
                        .mark_uploaded(job.id, &outcome.file_id, Some(outcome.message_id))
                        .await
                    {
                        tracing::error!(
                            "[REPLICATION] failed to mark replica {} uploaded: {e}",
                            job.id
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[REPLICATION] failed to replicate chunk {} into channel {}: {e}",
                        job.chunk_id,
                        job.channel_id
                    );
                    let _ = replicas_repo.mark_failed(job.id).await;
                    if is_chat_dead_error(&e) {
                        let _ = channels_repo.mark_dead(job.channel_id).await;
                    }
                }
            }
        }

        count
    }

    /// Spawn a background loop draining pending replicas. Sleeps `idle_interval` between
    /// batches whenever a batch turns up empty, otherwise loops immediately to drain backlog.
    pub fn spawn_loop(db: PgPool, base_url: String, rate_limit: u8, idle_interval: Duration) {
        tokio::spawn(async move {
            loop {
                let processed = Self::run_once(&db, &base_url, rate_limit).await;
                if processed == 0 {
                    tokio::time::sleep(idle_interval).await;
                }
            }
        });
    }
}
