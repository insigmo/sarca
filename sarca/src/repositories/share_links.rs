use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    errors::{SarcaError, SarcaResult},
    models::share_links::ShareLink,
};

pub const TABLE: &str = "share_links";

pub struct ShareLinksRepository<'d> {
    db: &'d PgPool,
}

impl<'d> ShareLinksRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            db,
        }
    }

    pub async fn create(
        &self,
        id: Uuid,
        token: &str,
        storage_id: Uuid,
        path: &str,
        created_by: Uuid,
        expires_at: Option<DateTime<Utc>>,
        password_hash: Option<&str>,
    ) -> SarcaResult<ShareLink> {
        sqlx::query_as(
            format!(
                "
                INSERT INTO {TABLE}
                    (id, token, storage_id, path, created_by, expires_at, password_hash)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING *
                "
            )
            .as_str(),
        )
        .bind(id)
        .bind(token)
        .bind(storage_id)
        .bind(path)
        .bind(created_by)
        .bind(expires_at)
        .bind(password_hash)
        .fetch_one(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })
    }

    pub async fn list_for_storage(&self, storage_id: Uuid) -> SarcaResult<Vec<ShareLink>> {
        sqlx::query_as(
            format!(
                "
                SELECT * FROM {TABLE}
                WHERE storage_id = $1
                  AND revoked_at IS NULL
                ORDER BY created_at DESC
                "
            )
            .as_str(),
        )
        .bind(storage_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })
    }

    pub async fn get_by_token(&self, token: &str) -> SarcaResult<ShareLink> {
        sqlx::query_as(
            format!(
                "
                SELECT * FROM {TABLE}
                WHERE token = $1
                "
            )
            .as_str(),
        )
        .bind(token)
        .fetch_optional(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?
        .ok_or_else(|| SarcaError::DoesNotExist("share link".to_owned()))
    }

    /// Soft-revoke. Idempotent if already revoked.
    pub async fn revoke(&self, id: Uuid, storage_id: Uuid) -> SarcaResult<()> {
        let res = sqlx::query(
            format!(
                "
                UPDATE {TABLE}
                SET revoked_at = COALESCE(revoked_at, NOW())
                WHERE id = $1 AND storage_id = $2
                "
            )
            .as_str(),
        )
        .bind(id)
        .bind(storage_id)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        if res.rows_affected() == 0 {
            return Err(SarcaError::DoesNotExist("share link".to_owned()));
        }
        Ok(())
    }

    /// Hard-delete share rows for a file or folder target (and descendants under a folder).
    /// Folder targets must end with `/`; file targets match the exact path only.
    pub async fn delete_for_target(&self, storage_id: Uuid, path: &str) -> SarcaResult<u64> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Ok(0);
        }

        let res = if path.ends_with('/') {
            sqlx::query(
                format!(
                    "
                    DELETE FROM {TABLE}
                    WHERE storage_id = $1
                      AND (path = $2 OR path LIKE $2 || '%')
                    "
                )
                .as_str(),
            )
            .bind(storage_id)
            .bind(path)
            .execute(self.db)
            .await
        } else {
            sqlx::query(
                format!(
                    "
                    DELETE FROM {TABLE}
                    WHERE storage_id = $1 AND path = $2
                    "
                )
                .as_str(),
            )
            .bind(storage_id)
            .bind(path)
            .execute(self.db)
            .await
        }
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(res.rows_affected())
    }

    /// Drop shares whose target path belongs to any of the given file rows
    /// (exact path match, or under a purged folder marker that ends with `/`).
    pub async fn delete_for_file_ids(&self, file_ids: &[Uuid]) -> SarcaResult<u64> {
        if file_ids.is_empty() {
            return Ok(0);
        }
        let res = sqlx::query(
            format!(
                "
                DELETE FROM {TABLE} sl
                WHERE EXISTS (
                    SELECT 1
                    FROM files f
                    WHERE f.id = ANY($1)
                      AND f.storage_id = sl.storage_id
                      AND (
                        sl.path = f.path
                        OR (
                          RIGHT(f.path, 1) = '/'
                          AND (sl.path = f.path OR sl.path LIKE f.path || '%')
                        )
                      )
                )
                "
            )
            .as_str(),
        )
        .bind(file_ids)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(res.rows_affected())
    }

    /// Rewrite share target paths after rename/move (mirrors file path rewrite).
    pub async fn rewrite_paths(
        &self,
        storage_id: Uuid,
        old_path: &str,
        new_path: &str,
    ) -> SarcaResult<()> {
        let old_path = old_path.trim_start_matches('/');
        let new_path = new_path.trim_start_matches('/');
        if old_path.is_empty() || old_path == new_path {
            return Ok(());
        }

        let is_folder = old_path.ends_with('/');
        if is_folder {
            let new_prefix =
                if new_path.ends_with('/') { new_path.to_string() } else { format!("{new_path}/") };
            let skip = old_path.len();
            sqlx::query(
                format!(
                    "
                    UPDATE {TABLE}
                    SET path = $1 || SUBSTRING(path FROM {skip} + 1)
                    WHERE storage_id = $2
                      AND (path = $3 OR path LIKE $3 || '%')
                    "
                )
                .as_str(),
            )
            .bind(&new_prefix)
            .bind(storage_id)
            .bind(old_path)
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;
        } else {
            // Exact file share, plus folder form if a folder was renamed without slash.
            let old_folder = format!("{old_path}/");
            let new_folder =
                if new_path.ends_with('/') { new_path.to_string() } else { format!("{new_path}/") };
            let skip = old_folder.len();

            sqlx::query(
                format!(
                    "
                    UPDATE {TABLE}
                    SET path = $1
                    WHERE storage_id = $2 AND path = $3
                    "
                )
                .as_str(),
            )
            .bind(new_path)
            .bind(storage_id)
            .bind(old_path)
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;

            sqlx::query(
                format!(
                    "
                    UPDATE {TABLE}
                    SET path = $1 || SUBSTRING(path FROM {skip} + 1)
                    WHERE storage_id = $2
                      AND (path = $3 OR path LIKE $3 || '%')
                    "
                )
                .as_str(),
            )
            .bind(&new_folder)
            .bind(storage_id)
            .bind(&old_folder)
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;
        }

        Ok(())
    }
}
