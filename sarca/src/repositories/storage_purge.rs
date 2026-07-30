use std::time::Duration;

use sqlx::{QueryBuilder, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::{
    errors::{SarcaError, SarcaResult},
    models::storage_purge::ClaimedPurgeMessage,
};

pub struct StoragePurgeRepository<'d> {
    db: &'d SqlitePool,
}

impl<'d> StoragePurgeRepository<'d> {
    pub fn new(db: &'d SqlitePool) -> Self {
        Self {
            db,
        }
    }

    pub async fn enqueue_in_tx(
        tx: &mut Transaction<'_, Sqlite>,
        job_id: Uuid,
        storage_id: Uuid,
        bot_token: &str,
        messages: &[(i64, i64)],
    ) -> SarcaResult<()> {
        sqlx::query(
            r#"
            INSERT INTO storage_purge_jobs (id, storage_id, bot_token)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(job_id)
        .bind(storage_id)
        .bind(bot_token)
        .execute(&mut **tx)
        .await
        .map_err(|e| {
            tracing::error!("storage purge enqueue job: {e}");
            SarcaError::Unknown
        })?;

        if messages.is_empty() {
            return Ok(());
        }

        QueryBuilder::new("INSERT INTO storage_purge_messages (job_id, chat_id, message_id) ")
            .push_values(messages, |mut q, (chat_id, message_id)| {
                q.push_bind(job_id).push_bind(chat_id).push_bind(message_id);
            })
            .build()
            .execute(&mut **tx)
            .await
            .map_err(|e| {
                tracing::error!("storage purge enqueue messages: {e}");
                SarcaError::Unknown
            })?;

        Ok(())
    }

    pub async fn claim_pending(&self, limit: i64) -> SarcaResult<Vec<ClaimedPurgeMessage>> {
        sqlx::query_as(
            r#"
            WITH cte AS (
              SELECT m.id
              FROM storage_purge_messages m
              JOIN storage_purge_jobs j ON j.id = m.job_id
              WHERE m.status = 'pending' AND j.completed_at IS NULL
              ORDER BY m.id
              FOR UPDATE OF m SKIP LOCKED
              LIMIT $1
            )
            UPDATE storage_purge_messages m
            SET status = 'in_progress', attempts = m.attempts + 1, updated_at = NOW()
            FROM cte, storage_purge_jobs j
            WHERE m.id = cte.id AND j.id = m.job_id
            RETURNING m.id, m.job_id, m.chat_id, m.message_id, j.bot_token, m.attempts
            "#,
        )
        .bind(limit)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("storage purge claim_pending: {e}");
            SarcaError::Unknown
        })
    }

    pub async fn requeue_stale_in_progress(&self, older_than: Duration) -> SarcaResult<u64> {
        let older_than_seconds = i64::try_from(older_than.as_secs()).unwrap_or(i64::MAX);
        let result = sqlx::query(
            r#"
            UPDATE storage_purge_messages
            SET status = 'pending', updated_at = NOW()
            WHERE status = 'in_progress'
              AND updated_at < NOW() - ($1 * INTERVAL '1 second')
            "#,
        )
        .bind(older_than_seconds)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("storage purge requeue_stale_in_progress: {e}");
            SarcaError::Unknown
        })?;
        Ok(result.rows_affected())
    }

    pub async fn mark_done(&self, id: i64) -> SarcaResult<()> {
        sqlx::query(
            r#"
            UPDATE storage_purge_messages
            SET status = 'done', last_error = NULL, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("storage purge mark_done: {e}");
            SarcaError::Unknown
        })?;
        Ok(())
    }

    pub async fn mark_retry(&self, id: i64, error: &str) -> SarcaResult<()> {
        sqlx::query(
            r#"
            UPDATE storage_purge_messages
            SET status = 'pending', last_error = $2, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(error)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("storage purge mark_retry: {e}");
            SarcaError::Unknown
        })?;
        Ok(())
    }

    pub async fn mark_failed(&self, id: i64, error: &str) -> SarcaResult<()> {
        sqlx::query(
            r#"
            UPDATE storage_purge_messages
            SET status = 'failed', last_error = $2, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(error)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("storage purge mark_failed: {e}");
            SarcaError::Unknown
        })?;
        Ok(())
    }

    pub async fn try_complete_jobs(&self) -> SarcaResult<u64> {
        let result = sqlx::query(
            r#"
            UPDATE storage_purge_jobs j
            SET completed_at = NOW()
            WHERE j.completed_at IS NULL
              AND NOT EXISTS (
                SELECT 1 FROM storage_purge_messages m
                WHERE m.job_id = j.id AND m.status IN ('pending', 'in_progress')
              )
            "#,
        )
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("storage purge try_complete_jobs: {e}");
            SarcaError::Unknown
        })?;
        Ok(result.rows_affected())
    }
}
