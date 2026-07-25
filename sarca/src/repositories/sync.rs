use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    errors::{SarcaError, SarcaResult},
    models::file_sync_events::{FileSyncEvent, SyncSnapshotEntry},
};

pub struct SyncRepository<'d> {
    db: &'d PgPool,
}

impl<'d> SyncRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            db,
        }
    }

    pub async fn changelog(
        &self,
        storage_id: Uuid,
        cursor: i64,
        limit: i64,
    ) -> SarcaResult<Vec<FileSyncEvent>> {
        sqlx::query_as::<_, FileSyncEvent>(
            r#"
            SELECT id, storage_id, file_id, path, op, size, is_file, content_hash, source_mtime, created_at
            FROM file_sync_events
            WHERE storage_id = $1 AND id > $2
            ORDER BY id ASC
            LIMIT $3
            "#,
        )
        .bind(storage_id)
        .bind(cursor)
        .bind(limit)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("sync changelog: {e}");
            SarcaError::Unknown
        })
    }

    pub async fn max_cursor(&self, storage_id: Uuid) -> SarcaResult<i64> {
        let row: (Option<i64>,) = sqlx::query_as(
            r#"
            SELECT MAX(id) FROM file_sync_events WHERE storage_id = $1
            "#,
        )
        .bind(storage_id)
        .fetch_one(self.db)
        .await
        .map_err(|e| {
            tracing::error!("sync max_cursor: {e}");
            SarcaError::Unknown
        })?;
        Ok(row.0.unwrap_or(0))
    }

    pub async fn snapshot(&self, storage_id: Uuid) -> SarcaResult<Vec<SyncSnapshotEntry>> {
        sqlx::query_as::<_, SyncSnapshotEntry>(
            r#"
            SELECT
              id AS file_id,
              path,
              size,
              (RIGHT(path, 1) <> '/') AS is_file,
              content_hash,
              source_mtime,
              updated_at
            FROM files
            WHERE storage_id = $1
              AND deleted_at IS NULL
              AND (is_uploaded OR RIGHT(path, 1) = '/')
            ORDER BY path ASC
            "#,
        )
        .bind(storage_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("sync snapshot: {e}");
            SarcaError::Unknown
        })
    }
}
