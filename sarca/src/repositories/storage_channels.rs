use sqlx::PgPool;
use uuid::Uuid;

use crate::common::db::errors::map_not_found;
use crate::common::types::ChatId;
use crate::errors::{SarcaError, SarcaResult};
use crate::models::storage_channels::{InStorageChannel, StorageChannel, CHANNEL_STATUS_DEAD};

pub const TABLE: &str = "storage_channels";

pub struct StorageChannelsRepository<'d> {
    db: &'d PgPool,
}

impl<'d> StorageChannelsRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self { db }
    }

    pub async fn list_by_storage(&self, storage_id: Uuid) -> SarcaResult<Vec<StorageChannel>> {
        sqlx::query_as(
            format!("SELECT * FROM {TABLE} WHERE storage_id = $1 ORDER BY position").as_str(),
        )
        .bind(storage_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| map_not_found(e, "storage channels"))
    }

    pub async fn get_by_id(&self, id: Uuid) -> SarcaResult<StorageChannel> {
        sqlx::query_as(format!("SELECT * FROM {TABLE} WHERE id = $1").as_str())
            .bind(id)
            .fetch_one(self.db)
            .await
            .map_err(|e| map_not_found(e, "storage channel"))
    }

    /// First free slot (1..=3) not currently used by `storage_id`, or `None` if all 3 taken.
    pub async fn next_free_position(&self, storage_id: Uuid) -> SarcaResult<Option<i16>> {
        let channels = self.list_by_storage(storage_id).await?;
        Ok((1i16..=3).find(|p| !channels.iter().any(|c| c.position == *p)))
    }

    pub async fn insert(&self, in_obj: InStorageChannel) -> SarcaResult<StorageChannel> {
        let id = Uuid::new_v4();

        sqlx::query(
            format!(
                "INSERT INTO {TABLE} (id, storage_id, position, chat_id, name, status)
                 VALUES ($1, $2, $3, $4, $5, $6)"
            )
            .as_str(),
        )
        .bind(id)
        .bind(in_obj.storage_id)
        .bind(in_obj.position)
        .bind(in_obj.chat_id)
        .bind(&in_obj.name)
        .bind(&in_obj.status)
        .execute(self.db)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(dbe) if dbe.is_foreign_key_violation() => {
                SarcaError::DoesNotExist("such storage".to_string())
            }
            sqlx::Error::Database(dbe) if dbe.is_unique_violation() => {
                SarcaError::StorageChatIdConflict
            }
            _ => {
                tracing::error!("{e}");
                SarcaError::Unknown
            }
        })?;

        Ok(StorageChannel {
            id,
            storage_id: in_obj.storage_id,
            position: in_obj.position,
            chat_id: in_obj.chat_id,
            name: in_obj.name,
            status: in_obj.status,
        })
    }

    pub async fn update_chat(
        &self,
        id: Uuid,
        chat_id: ChatId,
        name: &str,
    ) -> SarcaResult<StorageChannel> {
        sqlx::query_as(
            format!(
                "UPDATE {TABLE} SET chat_id = $2, name = $3, status = 'active' WHERE id = $1 RETURNING *"
            )
            .as_str(),
        )
        .bind(id)
        .bind(chat_id)
        .bind(name)
        .fetch_one(self.db)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(dbe) if dbe.is_unique_violation() => {
                SarcaError::StorageChatIdConflict
            }
            _ => map_not_found(e, "storage channel"),
        })
    }

    pub async fn update_name(&self, id: Uuid, name: &str) -> SarcaResult<StorageChannel> {
        sqlx::query_as(format!("UPDATE {TABLE} SET name = $2 WHERE id = $1 RETURNING *").as_str())
            .bind(id)
            .bind(name)
            .fetch_one(self.db)
            .await
            .map_err(|e| map_not_found(e, "storage channel"))
    }

    pub async fn mark_dead(&self, id: Uuid) -> SarcaResult<()> {
        sqlx::query(format!("UPDATE {TABLE} SET status = $2 WHERE id = $1").as_str())
            .bind(id)
            .bind(CHANNEL_STATUS_DEAD)
            .execute(self.db)
            .await
            .map_err(|_| SarcaError::Unknown)
            .map(|_| ())
    }

    pub async fn count_active(&self, storage_id: Uuid) -> SarcaResult<i64> {
        let row: (i64,) = sqlx::query_as(
            format!("SELECT COUNT(*) FROM {TABLE} WHERE storage_id = $1 AND status = 'active'")
                .as_str(),
        )
        .bind(storage_id)
        .fetch_one(self.db)
        .await
        .map_err(|_| SarcaError::Unknown)?;
        Ok(row.0)
    }

    pub async fn delete(&self, id: Uuid) -> SarcaResult<()> {
        sqlx::query(format!("DELETE FROM {TABLE} WHERE id = $1").as_str())
            .bind(id)
            .execute(self.db)
            .await
            .map_err(|e| map_not_found(e, "storage channel"))?;
        Ok(())
    }

    pub async fn list_all(&self) -> SarcaResult<Vec<StorageChannel>> {
        sqlx::query_as(format!("SELECT * FROM {TABLE}").as_str())
            .fetch_all(self.db)
            .await
            .map_err(|_| SarcaError::Unknown)
    }
}
