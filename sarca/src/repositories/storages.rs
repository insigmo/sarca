use sqlx::{Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::{
    common::db::errors::map_not_found,
    errors::{SarcaError, SarcaResult},
    models::storages::{InStorage, Storage, StorageWithInfo},
    repositories::{access::TABLE as ACCESS_TABLE, files::FILES_TABLE},
};

pub const TABLE: &str = "storages";

pub struct StoragesRepository<'d> {
    db: &'d SqlitePool,
}

impl<'d> StoragesRepository<'d> {
    pub fn new(db: &'d SqlitePool) -> Self {
        Self {
            db,
        }
    }

    pub async fn create(&self, in_obj: InStorage) -> SarcaResult<Storage> {
        let id = Uuid::new_v4();

        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {TABLE} (id, name, primary_position) VALUES ($1, $2, $3) "
        )))
        .bind(id)
        .bind(in_obj.name.clone())
        .bind(in_obj.primary_position)
        .execute(self.db)
        .await
        .map_err(|e| {
            match e {
                sqlx::Error::Database(dbe) if dbe.is_foreign_key_violation() => {
                    SarcaError::UserWasRemoved
                },
                _ => {
                    tracing::error!("{e}");
                    SarcaError::Unknown
                },
            }
        })?;

        let storage = Storage::new(id, in_obj.name, in_obj.primary_position);
        Ok(storage)
    }

    pub async fn list_by_user_id(&self, user_id: Uuid) -> SarcaResult<Vec<StorageWithInfo>> {
        tracing::debug!("[STORAGES REPO] Fetching storages for user_id={}", user_id);

        let result = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "
                SELECT
                    s.*,
                    COALESCE(t.files_amount, 0) AS files_amount,
                    COALESCE(t.size, 0) AS size,
                    EXISTS(
                        SELECT 1 FROM storage_channels sc
                        WHERE sc.storage_id = s.id AND sc.status = 'dead'
                    ) AS has_dead_channel
                FROM {TABLE} s
                JOIN {ACCESS_TABLE} a ON s.id = a.storage_id
                -- Totals first, then the join. Grouping the joined rows sorted
                -- every file through a temp b-tree on each storage list, and the
                -- covering files_totals_idx answers this without the table.
                LEFT JOIN (
                    SELECT storage_id, COUNT(*) AS files_amount, SUM(size) AS size
                    FROM {FILES_TABLE}
                    WHERE path NOT LIKE '%/'
                      AND is_uploaded = true
                      AND deleted_at IS NULL
                    GROUP BY storage_id
                ) t ON t.storage_id = s.id
                WHERE a.user_id = $1
                ORDER BY s.id
            "
        )))
        .bind(user_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| map_not_found(&e, "storages"))?;

        tracing::debug!("[STORAGES REPO] Found {} storages for user_id={}", result.len(), user_id);

        Ok(result)
    }

    /// Every storage id, for background jobs that walk all of them (the
    /// startup media warmer) rather than a single user's list.
    pub async fn list_all_ids(&self) -> SarcaResult<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT id FROM {TABLE}")))
                .fetch_all(self.db)
                .await
                .map_err(|e| {
                    tracing::error!("{e}");
                    SarcaError::Unknown
                })?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    pub async fn get_by_id(&self, id: Uuid) -> SarcaResult<Storage> {
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT * FROM {TABLE} WHERE id = $1")))
            .bind(id)
            .fetch_one(self.db)
            .await
            .map_err(|e| map_not_found(&e, "storage"))
    }

    pub async fn get_by_name_and_user_id(&self, name: &str, user_id: Uuid) -> SarcaResult<Storage> {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "
                SELECT s.* 
                FROM {TABLE} s
                JOIN {ACCESS_TABLE} a ON s.id = a.storage_id
                WHERE a.user_id = $1 AND s.name = $2
            "
        )))
        .bind(user_id)
        .bind(name)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(&e, "storage"))
    }

    pub async fn get_by_file_id(&self, file_id: Uuid) -> SarcaResult<Storage> {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT s.* FROM {TABLE} s JOIN {FILES_TABLE} AS f ON f.storage_id = s.id WHERE \
                 f.id = $1"
        )))
        .bind(file_id)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(&e, "storage"))
    }

    pub async fn update_name(&self, storage_id: Uuid, name: &str) -> SarcaResult<Storage> {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE {TABLE} SET name = $2 WHERE id = $1 RETURNING *"
        )))
        .bind(storage_id)
        .bind(name)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(&e, "storage"))
    }

    pub async fn set_primary_position(&self, storage_id: Uuid, position: i16) -> SarcaResult<()> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {TABLE} SET primary_position = $2 WHERE id = $1"
        )))
        .bind(storage_id)
        .bind(position)
        .execute(self.db)
        .await
        .map_err(|e| map_not_found(&e, "storage"))?;
        Ok(())
    }

    pub async fn delete_storage(&self, storage_id: Uuid) -> SarcaResult<()> {
        let mut tx = self.db.begin().await.map_err(|e| {
            tracing::error!("delete storage transaction begin: {e}");
            SarcaError::Unknown
        })?;
        self.delete_storage_in_tx(&mut tx, storage_id).await?;
        tx.commit().await.map_err(|e| {
            tracing::error!("delete storage transaction commit: {e}");
            SarcaError::Unknown
        })
    }

    /// Delete a storage and its dependent rows within an existing transaction.
    pub async fn delete_storage_in_tx(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        storage_id: Uuid,
    ) -> SarcaResult<()> {
        // storage_workers.storage_id has no ON DELETE, and a bot is bound 1:1 to a
        // storage — a detached worker is garbage (startup already sweeps those), so
        // drop it here instead of leaving a phantom bot until the next boot. The
        // purge job has already captured the token it needs for Telegram cleanup.
        sqlx::query("DELETE FROM storage_workers WHERE storage_id = $1")
            .bind(storage_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;

        // Delete files while the storage row still exists so the sync-event trigger
        // can record deletes (CASCADE-from-storage would leave storage already gone
        // and trip file_sync_events_storage_id_fkey).
        sqlx::query("DELETE FROM files WHERE storage_id = $1")
            .bind(storage_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;

        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {TABLE} WHERE id = $1")))
            .bind(storage_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| map_not_found(&e, "storage"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{
        common::db::pool::get_pool,
        models::{access::AccessType, files::InFile, users::InDBUser},
        repositories::{access::AccessRepository, files::FilesRepository, users::UsersRepository},
    };

    /// Totals count uploaded files only — not folder markers, not the trash, not
    /// unfinished uploads — and a storage with nothing in it still lists, at zero.
    #[tokio::test]
    async fn storage_list_totals_count_uploaded_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
        crate::startup::init_db(&pool).await;

        let user = UsersRepository::new(&pool)
            .create(InDBUser::new_password("u@sarca.test".into(), "x".into()))
            .await
            .unwrap();
        let (full, empty) = (Uuid::new_v4(), Uuid::new_v4());
        for (id, name) in [(full, "Full"), (empty, "Empty")] {
            sqlx::query("INSERT INTO storages (id, name, primary_position) VALUES ($1, $2, 1)")
                .bind(id)
                .bind(name)
                .execute(&pool)
                .await
                .unwrap();
            AccessRepository::new(&pool)
                .grant_for_user_id(id, user.id, AccessType::A)
                .await
                .unwrap();
        }

        let files = FilesRepository::new(&pool);
        for (path, size) in [("a.jpg", 10), ("dir/b.jpg", 20), ("dir/", 0), ("gone.jpg", 40)] {
            files.create_folder(InFile::new(path.into(), size, full)).await.unwrap();
        }
        files.delete("gone.jpg", full).await.unwrap();
        files.create_file_anyway(InFile::new("partial.mp4".into(), 80, full)).await.unwrap();

        let listed = StoragesRepository::new(&pool).list_by_user_id(user.id).await.unwrap();
        let totals = |id: Uuid| {
            listed.iter().find(|s| s.id == id).map(|s| (s.files_amount, s.size)).unwrap()
        };
        assert_eq!(listed.len(), 2);
        assert_eq!(totals(full), (2, 30));
        assert_eq!(totals(empty), (0, 0));
    }
}
