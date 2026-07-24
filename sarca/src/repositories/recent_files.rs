use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    errors::{SarcaError, SarcaResult},
    models::files::FSElement,
};

pub const TABLE: &str = "recent_files";
pub const FILES_TABLE: &str = "files";
pub const RECENT_LIMIT: i64 = 20;

pub struct RecentFilesRepository<'d> {
    db: &'d PgPool,
}

impl<'d> RecentFilesRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            db,
        }
    }

    /// Up to 20 live uploaded files, most recently viewed first.
    pub async fn list(&self, user_id: Uuid, storage_id: Uuid) -> SarcaResult<Vec<FSElement>> {
        let rows: Vec<(String, i64, bool)> = sqlx::query_as(
            format!(
                "
                SELECT
                    f.path,
                    f.size,
                    (f.thumb_telegram_file_id IS NOT NULL) AS has_thumb
                FROM {TABLE} rf
                JOIN {FILES_TABLE} f ON f.id = rf.file_id
                WHERE rf.user_id = $1
                  AND rf.storage_id = $2
                  AND f.deleted_at IS NULL
                  AND f.is_uploaded
                  AND f.path NOT LIKE '%/'
                ORDER BY rf.viewed_at DESC
                LIMIT {RECENT_LIMIT}
                "
            )
            .as_str(),
        )
        .bind(user_id)
        .bind(storage_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(rows
            .into_iter()
            .map(|(path, size, has_thumb)| {
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                FSElement {
                    path,
                    name,
                    size,
                    is_file: true,
                    has_thumb,
                }
            })
            .collect())
    }

    /// Upsert `viewed_at` and trim to the 20 most recent for (user, storage).
    pub async fn upsert_and_trim(
        &self,
        user_id: Uuid,
        storage_id: Uuid,
        file_id: Uuid,
    ) -> SarcaResult<()> {
        let mut tx = self.db.begin().await.map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        sqlx::query(
            format!(
                "
                INSERT INTO {TABLE} (user_id, storage_id, file_id, viewed_at)
                VALUES ($1, $2, $3, NOW())
                ON CONFLICT (user_id, file_id) DO UPDATE
                  SET viewed_at = NOW(),
                      storage_id = EXCLUDED.storage_id
                "
            )
            .as_str(),
        )
        .bind(user_id)
        .bind(storage_id)
        .bind(file_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        sqlx::query(
            format!(
                "
                DELETE FROM {TABLE} rf
                WHERE rf.user_id = $1
                  AND rf.storage_id = $2
                  AND rf.file_id NOT IN (
                      SELECT file_id FROM (
                          SELECT file_id
                          FROM {TABLE}
                          WHERE user_id = $1 AND storage_id = $2
                          ORDER BY viewed_at DESC
                          LIMIT {RECENT_LIMIT}
                      ) keep
                  )
                "
            )
            .as_str(),
        )
        .bind(user_id)
        .bind(storage_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        tx.commit().await.map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(())
    }
}
