use sqlx::{PgPool, QueryBuilder};
use uuid::Uuid;

use crate::common::types::ChatId;
use crate::errors::{SarcaError, SarcaResult};
use crate::models::chunk_replicas::{
    ChunkReplica, ReplicationStats, REPLICA_STATUS_FAILED, REPLICA_STATUS_PENDING,
    REPLICA_STATUS_UPLOADED,
};

pub const TABLE: &str = "chunk_replicas";

/// A pending/failed replica job joined with where it needs to land.
#[derive(Debug, sqlx::FromRow)]
pub struct PendingReplicaJob {
    pub id: Uuid,
    pub chunk_id: Uuid,
    pub channel_id: Uuid,
    pub target_chat_id: ChatId,
}

/// An existing uploaded replica usable as a copy/download source for a chunk.
#[derive(Debug, sqlx::FromRow)]
pub struct SourceReplica {
    pub telegram_file_id: Option<String>,
    pub telegram_message_id: Option<i64>,
    pub chat_id: ChatId,
    pub storage_id: Uuid,
}

pub struct ChunkReplicasRepository<'d> {
    db: &'d PgPool,
}

impl<'d> ChunkReplicasRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self { db }
    }

    pub async fn insert_batch(&self, replicas: Vec<ChunkReplica>) -> SarcaResult<()> {
        if replicas.is_empty() {
            return Ok(());
        }

        QueryBuilder::new(
            format!(
                "INSERT INTO {TABLE} (id, chunk_id, channel_id, telegram_file_id, telegram_message_id, status)"
            )
            .as_str(),
        )
        .push_values(replicas, |mut q, r| {
            q.push_bind(r.id)
                .push_bind(r.chunk_id)
                .push_bind(r.channel_id)
                .push_bind(r.telegram_file_id)
                .push_bind(r.telegram_message_id)
                .push_bind(r.status);
        })
        .build()
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(())
    }

    /// Pending or failed replicas whose target channel is still active.
    pub async fn list_pending(&self, limit: i64) -> SarcaResult<Vec<PendingReplicaJob>> {
        sqlx::query_as(
            format!(
                "
                SELECT cr.id, cr.chunk_id, cr.channel_id, sc.chat_id AS target_chat_id
                FROM {TABLE} cr
                JOIN storage_channels sc ON sc.id = cr.channel_id
                WHERE cr.status IN ('{REPLICA_STATUS_PENDING}', '{REPLICA_STATUS_FAILED}')
                  AND sc.status = 'active'
                ORDER BY cr.id
                LIMIT $1
                "
            )
            .as_str(),
        )
        .bind(limit)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })
    }

    /// An uploaded replica for `chunk_id` on an active channel other than `exclude_channel_id`,
    /// preferring one with a `telegram_message_id` (usable with `copyMessage`).
    pub async fn find_source_for_chunk(
        &self,
        chunk_id: Uuid,
        exclude_channel_id: Uuid,
    ) -> SarcaResult<Option<SourceReplica>> {
        sqlx::query_as(
            format!(
                "
                SELECT cr.telegram_file_id, cr.telegram_message_id, sc.chat_id, sc.storage_id
                FROM {TABLE} cr
                JOIN storage_channels sc ON sc.id = cr.channel_id
                WHERE cr.chunk_id = $1
                  AND cr.channel_id != $2
                  AND cr.status = '{REPLICA_STATUS_UPLOADED}'
                  AND sc.status = 'active'
                ORDER BY (cr.telegram_message_id IS NOT NULL) DESC
                LIMIT 1
                "
            )
            .as_str(),
        )
        .bind(chunk_id)
        .bind(exclude_channel_id)
        .fetch_optional(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })
    }

    pub async fn mark_uploaded(
        &self,
        id: Uuid,
        telegram_file_id: &str,
        telegram_message_id: Option<i64>,
    ) -> SarcaResult<()> {
        sqlx::query(
            format!(
                "UPDATE {TABLE} SET telegram_file_id = $2, telegram_message_id = $3, status = '{REPLICA_STATUS_UPLOADED}' WHERE id = $1"
            )
            .as_str(),
        )
        .bind(id)
        .bind(telegram_file_id)
        .bind(telegram_message_id)
        .execute(self.db)
        .await
        .map_err(|_| SarcaError::Unknown)
        .map(|_| ())
    }

    pub async fn mark_failed(&self, id: Uuid) -> SarcaResult<()> {
        sqlx::query(format!("UPDATE {TABLE} SET status = '{REPLICA_STATUS_FAILED}' WHERE id = $1").as_str())
            .bind(id)
            .execute(self.db)
            .await
            .map_err(|_| SarcaError::Unknown)
            .map(|_| ())
    }

    /// Queue every chunk of `storage_id`'s files for replication into `channel_id`
    /// (used for catch-up on a new or repaired channel). No-op for chunks already queued/replicated.
    pub async fn enqueue_for_channel(&self, storage_id: Uuid, channel_id: Uuid) -> SarcaResult<()> {
        let chunk_ids: Vec<(Uuid,)> = sqlx::query_as(
            "
            SELECT fc.id
            FROM file_chunks fc
            JOIN files f ON f.id = fc.file_id
            WHERE f.storage_id = $1
            ",
        )
        .bind(storage_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        if chunk_ids.is_empty() {
            return Ok(());
        }

        let replicas: Vec<ChunkReplica> = chunk_ids
            .into_iter()
            .map(|(chunk_id,)| ChunkReplica::new_pending(Uuid::new_v4(), chunk_id, channel_id))
            .collect();

        let mut builder = QueryBuilder::new(
            format!(
                "INSERT INTO {TABLE} (id, chunk_id, channel_id, telegram_file_id, telegram_message_id, status)"
            )
            .as_str(),
        );
        builder.push_values(replicas, |mut q, r| {
            q.push_bind(r.id)
                .push_bind(r.chunk_id)
                .push_bind(r.channel_id)
                .push_bind(r.telegram_file_id)
                .push_bind(r.telegram_message_id)
                .push_bind(r.status);
        });
        builder.push(" ON CONFLICT (chunk_id, channel_id) DO NOTHING");
        builder
            .build()
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;

        Ok(())
    }

    pub async fn retry_failed(&self, storage_id: Uuid) -> SarcaResult<()> {
        sqlx::query(
            format!(
                "
                UPDATE {TABLE} SET status = '{REPLICA_STATUS_PENDING}'
                WHERE status = '{REPLICA_STATUS_FAILED}'
                  AND channel_id IN (SELECT id FROM storage_channels WHERE storage_id = $1)
                "
            )
            .as_str(),
        )
        .bind(storage_id)
        .execute(self.db)
        .await
        .map_err(|_| SarcaError::Unknown)
        .map(|_| ())
    }

    pub async fn replication_stats(&self, storage_id: Uuid) -> SarcaResult<ReplicationStats> {
        sqlx::query_as(
            format!(
                "
                SELECT
                    COUNT(*) FILTER (WHERE cr.status = '{REPLICA_STATUS_PENDING}') AS pending,
                    COUNT(*) FILTER (WHERE cr.status = '{REPLICA_STATUS_UPLOADED}') AS uploaded,
                    COUNT(*) FILTER (WHERE cr.status = '{REPLICA_STATUS_FAILED}') AS failed
                FROM {TABLE} cr
                JOIN storage_channels sc ON sc.id = cr.channel_id
                WHERE sc.storage_id = $1
                "
            )
            .as_str(),
        )
        .bind(storage_id)
        .fetch_one(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })
    }
}
