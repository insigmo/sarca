use sqlx::PgPool;
use uuid::Uuid;

use crate::common::db::errors::map_not_found;
use crate::errors::{SarcaError, SarcaResult};
use crate::models::storages::{InStorage, Storage, StorageWithInfo};
use crate::repositories::{access::TABLE as ACCESS_TABLE, files::FILES_TABLE};

pub const TABLE: &str = "storages";

pub struct StoragesRepository<'d> {
    db: &'d PgPool,
}

impl<'d> StoragesRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self { db }
    }

    pub async fn create(&self, in_obj: InStorage) -> SarcaResult<Storage> {
        let id = Uuid::new_v4();

        sqlx::query(
            format!(
                "INSERT INTO {TABLE} (id, name, primary_position) VALUES ($1, $2, $3)"
            )
            .as_str(),
        )
        .bind(id)
        .bind(in_obj.name.clone())
        .bind(in_obj.primary_position)
        .execute(self.db)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(dbe) if dbe.is_foreign_key_violation() => {
                SarcaError::UserWasRemoved
            }
            _ => {
                tracing::error!("{e}");
                SarcaError::Unknown
            }
        })?;

        let storage = Storage::new(id, in_obj.name, in_obj.primary_position);
        Ok(storage)
    }

    pub async fn list_by_user_id(&self, user_id: Uuid) -> SarcaResult<Vec<StorageWithInfo>> {
        tracing::debug!(
            "[STORAGES REPO] Fetching storages for user_id={}",
            user_id
        );
        
        let result = sqlx::query_as(
            format!(
                "
                SELECT
                    s.*,
                    COUNT(f.id) AS files_amount,
                    COALESCE(SUM(f.size), 0)::BigInt as size,
                    EXISTS(
                        SELECT 1 FROM storage_channels sc
                        WHERE sc.storage_id = s.id AND sc.status = 'dead'
                    ) AS has_dead_channel
                FROM {TABLE} s
                JOIN {ACCESS_TABLE} a ON s.id = a.storage_id
                LEFT JOIN {FILES_TABLE} f ON s.id = f.storage_id
                    AND f.path NOT LIKE '%/'
                    AND f.is_uploaded = true
                    AND f.deleted_at IS NULL
                WHERE a.user_id = $1
                GROUP by s.id
            "
            )
            .as_str(),
        )
        .bind(user_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| map_not_found(e, "storages"))?;
        
        tracing::debug!(
            "[STORAGES REPO] Found {} storages for user_id={}",
            result.len(),
            user_id
        );
        
        Ok(result)
    }

    pub async fn get_by_id(&self, id: Uuid) -> SarcaResult<Storage> {
        sqlx::query_as(format!("SELECT * FROM {TABLE} WHERE id = $1").as_str())
            .bind(id)
            .fetch_one(self.db)
            .await
            .map_err(|e| map_not_found(e, "storage"))
    }

    pub async fn get_by_name_and_user_id(
        &self,
        name: &str,
        user_id: Uuid,
    ) -> SarcaResult<Storage> {
        sqlx::query_as(
            format!(
                "
                SELECT s.* 
                FROM {TABLE} s
                JOIN {ACCESS_TABLE} a ON s.id = a.storage_id
                WHERE a.user_id = $1 AND s.name = $2
            "
            )
            .as_str(),
        )
        .bind(user_id)
        .bind(name)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(e, "storage"))
    }

    pub async fn get_by_file_id(&self, file_id: Uuid) -> SarcaResult<Storage> {
        sqlx::query_as(
            format!("SELECT s.* FROM {TABLE} s JOIN {FILES_TABLE} AS f ON f.storage_id = s.id WHERE f.id = $1").as_str(),
        )
        .bind(file_id)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(e, "storage"))
    }

    pub async fn update_name(&self, storage_id: Uuid, name: &str) -> SarcaResult<Storage> {
        sqlx::query_as(
            format!("UPDATE {TABLE} SET name = $2 WHERE id = $1 RETURNING *").as_str(),
        )
        .bind(storage_id)
        .bind(name)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(e, "storage"))
    }

    pub async fn set_primary_position(&self, storage_id: Uuid, position: i16) -> SarcaResult<()> {
        sqlx::query(
            format!("UPDATE {TABLE} SET primary_position = $2 WHERE id = $1").as_str(),
        )
        .bind(storage_id)
        .bind(position)
        .execute(self.db)
        .await
        .map_err(|e| map_not_found(e, "storage"))?;
        Ok(())
    }

    pub async fn delete_storage(&self, storage_id: Uuid) -> SarcaResult<()> {
        // storage_workers.storage_id has no ON DELETE; detach first
        sqlx::query(
            "UPDATE storage_workers SET storage_id = NULL WHERE storage_id = $1",
        )
        .bind(storage_id)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        sqlx::query(format!("DELETE FROM {TABLE} WHERE id = $1").as_str())
            .bind(storage_id)
            .execute(self.db)
            .await
            .map_err(|e| map_not_found(e, "storage"))?;
        Ok(())
    }
}
