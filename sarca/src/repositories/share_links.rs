use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{errors::{SarcaError, SarcaResult}, models::share_links::ShareLink};

pub const TABLE: &str = "share_links";

pub struct ShareLinksRepository<'d> {
    db: &'d PgPool,
}

impl<'d> ShareLinksRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self { db }
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

    #[allow(dead_code)]
    pub async fn get_by_id(&self, id: Uuid, storage_id: Uuid) -> SarcaResult<ShareLink> {
        sqlx::query_as(
            format!(
                "
                SELECT * FROM {TABLE}
                WHERE id = $1 AND storage_id = $2
                "
            )
            .as_str(),
        )
        .bind(id)
        .bind(storage_id)
        .fetch_optional(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?
        .ok_or_else(|| SarcaError::DoesNotExist("share link".to_owned()))
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
}
