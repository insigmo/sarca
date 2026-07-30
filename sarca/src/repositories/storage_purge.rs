use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
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
        let mut conn = self.db.acquire().await.map_err(|e| {
            tracing::error!("storage purge claim_pending acquire: {e}");
            SarcaError::Unknown
        })?;

        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                tracing::error!("storage purge claim_pending begin: {e}");
                SarcaError::Unknown
            })?;

        let result = async {
            let ids: Vec<(i64,)> = sqlx::query_as(
                r#"
                SELECT m.id
                FROM storage_purge_messages m
                JOIN storage_purge_jobs j ON j.id = m.job_id
                WHERE m.status = 'pending' AND j.completed_at IS NULL
                ORDER BY m.id
                LIMIT $1
                "#,
            )
            .bind(limit)
            .fetch_all(&mut *conn)
            .await
            .map_err(|e| {
                tracing::error!("storage purge claim_pending select: {e}");
                SarcaError::Unknown
            })?;

            if ids.is_empty() {
                return Ok(vec![]);
            }

            let id_list: Vec<i64> = ids.into_iter().map(|(id,)| id).collect();

            let mut update = QueryBuilder::new(
                "UPDATE storage_purge_messages SET status = 'in_progress', attempts = attempts + 1, updated_at = datetime('now') WHERE id IN (",
            );
            update.push_values(id_list.iter(), |mut q, id| {
                q.push_bind(id);
            });
            update.push(")");
            update.build().execute(&mut *conn).await.map_err(|e| {
                tracing::error!("storage purge claim_pending update: {e}");
                SarcaError::Unknown
            })?;

            let mut fetch = QueryBuilder::new(
                r#"
                SELECT m.id, m.job_id, m.chat_id, m.message_id, j.bot_token, m.attempts
                FROM storage_purge_messages m
                JOIN storage_purge_jobs j ON j.id = m.job_id
                WHERE m.id IN (
                "#,
            );
            fetch.push_values(id_list.iter(), |mut q, id| {
                q.push_bind(id);
            });
            fetch.push(")");

            fetch
                .build_query_as::<ClaimedPurgeMessage>()
                .fetch_all(&mut *conn)
                .await
                .map_err(|e| {
                    tracing::error!("storage purge claim_pending fetch: {e}");
                    SarcaError::Unknown
                })
        }
        .await;

        match result {
            Ok(rows) => {
                sqlx::query("COMMIT").execute(&mut *conn).await.map_err(|e| {
                    tracing::error!("storage purge claim_pending commit: {e}");
                    SarcaError::Unknown
                })?;
                Ok(rows)
            },
            Err(e) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(e)
            },
        }
    }

    pub async fn requeue_stale_in_progress(&self, older_than: Duration) -> SarcaResult<u64> {
        let cutoff = Utc::now()
            - ChronoDuration::from_std(older_than).unwrap_or(ChronoDuration::seconds(i64::MAX));
        let result = sqlx::query(
            r#"
            UPDATE storage_purge_messages
            SET status = 'pending', updated_at = datetime('now')
            WHERE status = 'in_progress'
              AND updated_at < $1
            "#,
        )
        .bind(cutoff)
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
            SET status = 'done', last_error = NULL, updated_at = datetime('now')
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
            SET status = 'pending', last_error = $2, updated_at = datetime('now')
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
            SET status = 'failed', last_error = $2, updated_at = datetime('now')
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
            SET completed_at = datetime('now')
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
