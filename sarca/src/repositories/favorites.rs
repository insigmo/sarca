use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    errors::{SarcaError, SarcaResult},
    models::files::FSElement,
};

pub const TABLE: &str = "favorites";
pub const FILES_TABLE: &str = "files";

pub struct FavoritesRepository<'d> {
    db: &'d PgPool,
}

impl<'d> FavoritesRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            db,
        }
    }

    /// Live uploaded files starred by the user in this storage, newest star first.
    pub async fn list(&self, user_id: Uuid, storage_id: Uuid) -> SarcaResult<Vec<FSElement>> {
        let rows: Vec<(String, i64, bool)> = sqlx::query_as(
            format!(
                "
                SELECT
                    f.path,
                    f.size,
                    (f.thumb_telegram_file_id IS NOT NULL) AS has_thumb
                FROM {TABLE} fav
                JOIN {FILES_TABLE} f ON f.id = fav.file_id
                WHERE fav.user_id = $1
                  AND fav.storage_id = $2
                  AND f.deleted_at IS NULL
                  AND f.is_uploaded
                  AND f.path NOT LIKE '%/'
                ORDER BY fav.created_at DESC
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

    /// Idempotent star. Returns Ok even if already starred.
    pub async fn add(&self, user_id: Uuid, storage_id: Uuid, file_id: Uuid) -> SarcaResult<()> {
        sqlx::query(
            format!(
                "
                INSERT INTO {TABLE} (user_id, storage_id, file_id)
                VALUES ($1, $2, $3)
                ON CONFLICT (user_id, file_id) DO NOTHING
                "
            )
            .as_str(),
        )
        .bind(user_id)
        .bind(storage_id)
        .bind(file_id)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(())
    }

    /// Unstar by path. Idempotent (no error if not starred).
    pub async fn remove_by_path(
        &self,
        user_id: Uuid,
        storage_id: Uuid,
        path: &str,
    ) -> SarcaResult<()> {
        sqlx::query(
            format!(
                "
                DELETE FROM {TABLE} fav
                USING {FILES_TABLE} f
                WHERE fav.file_id = f.id
                  AND fav.user_id = $1
                  AND fav.storage_id = $2
                  AND f.path = $3
                "
            )
            .as_str(),
        )
        .bind(user_id)
        .bind(storage_id)
        .bind(path)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(())
    }
}
