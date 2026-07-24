use std::path::Path;

use sqlx::{PgPool, QueryBuilder};
use uuid::Uuid;

use crate::common::db::errors::map_not_found;
use crate::errors::{SarcaError, SarcaResult};
use crate::models::file_chunks::{FileChunk, FileChunkWithReplica};
use crate::models::files::{DBFSElement, FSElement, File, InFile, SearchFSElement};

pub const FILES_TABLE: &str = "files";
pub const CHUNKS_TABLE: &str = "file_chunks";

/// General repo for files and chunks since they share common logic
pub struct FilesRepository<'d> {
    db: &'d PgPool,
}

impl<'d> FilesRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self { db }
    }

    pub async fn create_folder(&self, in_obj: InFile) -> SarcaResult<File> {
        self._create_file(in_obj, true).await
    }

    async fn _create_file(&self, in_obj: InFile, is_uploaded: bool) -> SarcaResult<File> {
        let id = Uuid::new_v4();

        sqlx::query(
            format!(
                "
                INSERT INTO {FILES_TABLE} (id, path, size, storage_id, is_uploaded, chunk_size_bytes)
                VALUES ($1, $2, $3, $4, $5, $6);
            "
            )
            .as_str(),
        )
        .bind(id)
        .bind(&in_obj.path)
        .bind(in_obj.size)
        .bind(in_obj.storage_id)
        .bind(is_uploaded)
        .bind(in_obj.chunk_size_bytes)
        .execute(self.db)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(dbe) if dbe.is_foreign_key_violation() => {
                SarcaError::DoesNotExist("such storage".to_string())
            }
            sqlx::Error::Database(dbe) if dbe.is_unique_violation() => {
                SarcaError::AlreadyExists("File with such name".to_string())
            }
            _ => {
                tracing::error!("{e}");
                SarcaError::Unknown
            }
        })?;

        let storage = File::new(
            id,
            in_obj.path,
            in_obj.size,
            in_obj.storage_id,
            false,
            in_obj.chunk_size_bytes,
        );
        Ok(storage)
    }

    /// Creates a file even if the given path already exists
    pub async fn create_file_anyway(&self, in_obj: InFile) -> SarcaResult<File> {
        let id = Uuid::new_v4();

        // lol/kek/sdf.nj.dskf/sdkl.fdsklf/lol .kek.dsf
        let (path_with_stem, suffix) = {
            let mut splited_path: Vec<_> = in_obj.path.split("/").collect();
            let last = splited_path.last_mut().unwrap();
            let (stem, suffix) = last
                .split_once(".")
                .map(|(stem, suffix)| (stem, format!(".{suffix}")))
                .unwrap_or((last, "".to_owned()));
            *last = stem;
            (splited_path.join("/"), suffix)
        };

        let chars_to_skip = path_with_stem.len() + 3; // if the name is `kek` then it's gonna be a len of `kek (` + 1
        let skip_chars_from_back = chars_to_skip + suffix.len();

        // https://www.db-fiddle.com/f/i6XCvTSi5cpAVu5AAfiNqm/16
        sqlx::query_as(
            format!(
                r#"
                INSERT INTO files (path, storage_id, id, size, is_uploaded, chunk_size_bytes)
                WITH f AS (
                    SELECT path
                    FROM {FILES_TABLE}
                    WHERE storage_id = $3 AND deleted_at IS NULL AND path ~ ('^(' || regexp_quote($1) || regexp_quote($2) || '|' || regexp_quote($1) || ' \(\d+\)' || regexp_quote($2) || ')$')
                    ORDER BY path DESC
                )
                SELECT
                    CASE
                        WHEN NOT EXISTS(
                            SELECT path
                            FROM f
                            WHERE path = $1 || $2
                        ) THEN $1 || $2
                        ELSE
                            CASE
                                WHEN COUNT(f) > 1 THEN (
                                    WITH cte AS (
                                        SELECT *
                                        FROM (
                                            SELECT SUBSTRING(f.path, {chars_to_skip}, LENGTH(f.path) - {skip_chars_from_back})::numeric AS i
                                            FROM f
                                            WHERE f.path != $1 || $2
                                        ) AS n
                                        WHERE i > 0
                                        ORDER BY i
                                    )
                                    SELECT $1 || ' (' || COALESCE(t.next_i, (
                                        SELECT cte.i + 1
                                        FROM cte
                                        ORDER BY cte.i DESC
                                        LIMIT 1
                                    )) || ')' || $2
                                    FROM cte
                                    FULL OUTER JOIN (
                                        SELECT prev_i + 1 AS next_i
                                        FROM (
                                            SELECT LAG(i, 1, 0) OVER() AS prev_i, i
                                            FROM cte
                                        ) t
                                        WHERE prev_i != t.i - 1
                                        LIMIT 1
                                    ) t ON cte.i = t.next_i
                                    LIMIT 1
                                )
                                WHEN COUNT(f) = 1 THEN $1 || ' (1)' || $2
                                ELSE $1 || $2
                            END
                    END,
                    $3,
                    $4,
                    $5,
                    false,
                    $6
                FROM f
                RETURNING *;
            "#
            )
            .as_str(),
        )
        .bind(&path_with_stem)
        .bind(&suffix)
        .bind(in_obj.storage_id)
        .bind(id)
        .bind(in_obj.size)
        .bind(in_obj.chunk_size_bytes)
        .fetch_one(self.db)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(dbe) if dbe.is_foreign_key_violation() => {
                SarcaError::DoesNotExist("such storage".to_string())
            }
            _ => {
                tracing::error!("{e}");
                SarcaError::Unknown
            }
        })
    }

    pub async fn create_chunks_batch(&self, chunks: Vec<FileChunk>) -> SarcaResult<()> {
        QueryBuilder::new(
            format!("INSERT INTO {CHUNKS_TABLE} (id, file_id, position)").as_str(),
        )
        .push_values(chunks, |mut q, chunk| {
            q.push_bind(chunk.id)
                .push_bind(chunk.file_id)
                .push_bind(chunk.position);
        })
        .build()
        .execute(self.db)
        .await
        .map_err(|_| SarcaError::Unknown)?;

        Ok(())
    }

    /// Chunks of `file_id` that have an `uploaded` replica on `channel_id`, ordered by position.
    /// Length may be less than the file's total chunk count if that channel doesn't (yet)
    /// have every chunk replicated.
    pub async fn list_chunks_with_replica_for_channel(
        &self,
        file_id: Uuid,
        channel_id: Uuid,
    ) -> SarcaResult<Vec<FileChunkWithReplica>> {
        sqlx::query_as(
            format!(
                "
                SELECT fc.position, cr.telegram_file_id
                FROM {CHUNKS_TABLE} fc
                JOIN chunk_replicas cr ON cr.chunk_id = fc.id
                    AND cr.channel_id = $2
                    AND cr.status = 'uploaded'
                    AND cr.telegram_file_id IS NOT NULL
                WHERE fc.file_id = $1
                ORDER BY fc.position
                "
            )
            .as_str(),
        )
        .bind(file_id)
        .bind(channel_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })
    }

    /// NOTE:
    ///
    /// `prefix` must be without leading and trailing slashes
    pub async fn list_dir(
        &self,
        storage_id: Uuid,
        prefix: &str,
    ) -> SarcaResult<Vec<FSElement>> {
        let query = {
            let adding_to_position = !prefix.is_empty() as usize + 1;
            let split_position = prefix.matches("/").count() + adding_to_position;
            let split_part = format!("SPLIT_PART(path, '/', {split_position})");
            let path_filter = if prefix.is_empty() {
                ""
            } else {
                "AND path LIKE $1 || '%'"
            };

            format!(
                "
                SELECT
                    DISTINCT {split_part} AS name,
                    $1 || {split_part} = path AS is_file,
                    CASE
                        WHEN $1 || {split_part} = path THEN size
                        ELSE (SELECT SUM(size) FROM {FILES_TABLE} WHERE deleted_at IS NULL AND path LIKE $1 || {split_part} || '/' || '%')::BigInt
                    END AS size,
                    CASE
                        WHEN $1 || {split_part} = path THEN (thumb_telegram_file_id IS NOT NULL)
                        ELSE false
                    END AS has_thumb
                FROM {FILES_TABLE}
                WHERE storage_id = $2 {path_filter} AND is_uploaded AND deleted_at IS NULL AND {split_part} <> '';
            "
            )
        };

        let prefix = if prefix.is_empty() {
            prefix.to_string()
        } else {
            format!("{prefix}/")
        };

        let fs_layer = sqlx::query_as::<_, DBFSElement>(&query)
            .bind(&prefix)
            .bind(storage_id)
            .fetch_all(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;
        let fs_layer = fs_layer
            .into_iter()
            .map(|el| {
                let path = format!("{prefix}{}", el.name);
                FSElement {
                    path,
                    name: el.name,
                    is_file: el.is_file,
                    size: el.size,
                    has_thumb: el.has_thumb,
                }
            })
            .collect();

        Ok(fs_layer)
    }

    pub async fn search(
        &self,
        search_path: &str,
        path: &str,
        storage_id: Uuid,
    ) -> SarcaResult<Vec<SearchFSElement>> {
        sqlx::query_as(
            format!(
                "SELECT
                    path,
                    path NOT LIKE '%/' AS is_file
                FROM {FILES_TABLE}
                WHERE storage_id = $1 AND deleted_at IS NULL AND path ILIKE $2 || '%' || $3 || '%'
            "
            )
            .as_str(),
        )
        .bind(storage_id)
        .bind(path)
        .bind(search_path)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })
    }

    pub async fn get_file_by_path(&self, path: &str, storage_id: Uuid) -> SarcaResult<File> {
        sqlx::query_as(
            format!(
                "SELECT * FROM {FILES_TABLE} WHERE storage_id = $1 AND path = $2 AND deleted_at IS NULL"
            )
            .as_str(),
        )
        .bind(storage_id)
        .bind(path)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(e, "file"))
    }

    /// Sum of uploaded file sizes under a folder prefix (prefix must end with `/`).
    pub async fn sum_uploaded_size_under(
        &self,
        storage_id: Uuid,
        folder_prefix: &str,
    ) -> SarcaResult<i64> {
        let row: (i64,) = sqlx::query_as(
            format!(
                "
                SELECT COALESCE(SUM(size), 0)::BigInt
                FROM {FILES_TABLE}
                WHERE storage_id = $1
                  AND is_uploaded
                  AND deleted_at IS NULL
                  AND path LIKE $2 || '%'
                  AND path NOT LIKE '%/';
            "
            )
            .as_str(),
        )
        .bind(storage_id)
        .bind(folder_prefix)
        .fetch_one(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(row.0)
    }

    /// Uploaded files (not folder markers) under a folder prefix (prefix must end with `/`).
    pub async fn list_uploaded_files_under(
        &self,
        storage_id: Uuid,
        folder_prefix: &str,
    ) -> SarcaResult<Vec<File>> {
        sqlx::query_as(
            format!(
                "
                SELECT *
                FROM {FILES_TABLE}
                WHERE storage_id = $1
                  AND is_uploaded
                  AND deleted_at IS NULL
                  AND path LIKE $2 || '%'
                  AND path NOT LIKE '%/'
                ORDER BY path
            "
            )
            .as_str(),
        )
        .bind(storage_id)
        .bind(folder_prefix)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })
    }

    pub async fn get_by_id(&self, id: Uuid) -> SarcaResult<File> {
        sqlx::query_as(format!("SELECT * FROM {FILES_TABLE} WHERE id = $1").as_str())
            .bind(id)
            .fetch_one(self.db)
            .await
            .map_err(|e| map_not_found(e, "file"))
    }

    pub async fn set_thumb(
        &self,
        file_id: Uuid,
        thumb_telegram_file_id: &str,
        thumb_telegram_message_id: i64,
    ) -> SarcaResult<()> {
        sqlx::query(
            format!(
                "
                UPDATE {FILES_TABLE}
                SET thumb_telegram_file_id = $2,
                    thumb_telegram_message_id = $3
                WHERE id = $1
                "
            )
            .as_str(),
        )
        .bind(file_id)
        .bind(thumb_telegram_file_id)
        .bind(thumb_telegram_message_id)
        .execute(self.db)
        .await
        .map_err(|_| SarcaError::Unknown)
        .map(|_| ())
    }

    /// `(chat_id, message_id, storage_id)` for thumbnail Telegram messages of the given files.
    pub async fn list_thumb_messages_for_files(
        &self,
        file_ids: &[Uuid],
    ) -> SarcaResult<Vec<(i64, i64, Uuid)>> {
        if file_ids.is_empty() {
            return Ok(vec![]);
        }
        // Thumb is uploaded to the primary channel at upload time; try all storage
        // channels so purge still works if primary later rotated.
        let rows: Vec<(i64, Option<i64>, Uuid)> = sqlx::query_as(
            format!(
                "
                SELECT sc.chat_id, f.thumb_telegram_message_id, f.storage_id
                FROM {FILES_TABLE} f
                JOIN storage_channels sc ON sc.storage_id = f.storage_id
                WHERE f.id = ANY($1)
                  AND f.thumb_telegram_message_id IS NOT NULL
                "
            )
            .as_str(),
        )
        .bind(file_ids)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(rows
            .into_iter()
            .filter_map(|(chat_id, message_id, storage_id)| {
                message_id.map(|mid| (chat_id, mid, storage_id))
            })
            .collect())
    }

    pub async fn list_chunks_of_file(&self, file_id: Uuid) -> SarcaResult<Vec<FileChunk>> {
        sqlx::query_as(format!("SELECT * FROM {CHUNKS_TABLE} WHERE file_id = $1").as_str())
            .bind(file_id)
            .fetch_all(self.db)
            .await
            .map_err(|e| map_not_found(e, "file chunks"))
    }

    pub async fn set_as_uploaded(&self, file_id: Uuid) -> SarcaResult<()> {
        sqlx::query(format!("UPDATE {FILES_TABLE} SET is_uploaded = true WHERE id = $1").as_str())
            .bind(file_id)
            .execute(self.db)
            .await
            .map_err(|_| SarcaError::Unknown)
            .map(|_| ())
    }

    pub async fn delete_with_folders(&self, id: Uuid) -> SarcaResult<()> {
        sqlx::query(format!("DELETE FROM {FILES_TABLE} WHERE id = $1").as_str())
            .bind(id)
            .execute(self.db)
            .await
            .map_err(|_| SarcaError::Unknown)
            .map(|_| ())
    }

    pub async fn delete(&self, path: &str, storage_id: Uuid) -> SarcaResult<()> {
        let mut transaction = self.db.begin().await.map_err(|e| map_not_found(e, ""))?;

        // Folders may arrive without a trailing slash from the UI.
        let is_folder = path.ends_with('/');
        let folder_prefix = if is_folder {
            path.to_string()
        } else {
            // Treat as folder when a live folder marker or children exist under path/
            let probe = format!("{path}/");
            let has_folder: (bool,) = sqlx::query_as(&format!(
                "
                SELECT EXISTS(
                    SELECT 1 FROM {FILES_TABLE}
                    WHERE storage_id = $1
                      AND deleted_at IS NULL
                      AND (path = $2 OR path LIKE $2 || '%')
                )
                "
            ))
            .bind(storage_id)
            .bind(&probe)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|e| map_not_found(e, "file"))?;

            if has_folder.0 {
                probe
            } else {
                String::new()
            }
        };

        let affected = if !folder_prefix.is_empty() {
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE}
                SET deleted_at = NOW()
                WHERE storage_id = $1
                  AND deleted_at IS NULL
                  AND (path = $2 OR path LIKE $2 || '%');
                "
            ))
            .bind(storage_id)
            .bind(&folder_prefix)
            .execute(&mut *transaction)
            .await
            .map_err(|e| map_not_found(e, "file"))?
            .rows_affected()
        } else {
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE}
                SET deleted_at = NOW()
                WHERE storage_id = $1 AND deleted_at IS NULL AND path = $2;
                "
            ))
            .bind(storage_id)
            .bind(path)
            .execute(&mut *transaction)
            .await
            .map_err(|e| map_not_found(e, "file"))?
            .rows_affected()
        };

        if affected == 0 {
            return Err(SarcaError::DoesNotExist("file".to_string()));
        }

        // Recreate parent folder marker only for non-root parents that became empty.
        let deleted_path = if !folder_prefix.is_empty() {
            folder_prefix.trim_end_matches('/').to_string()
        } else {
            path.to_string()
        };
        if let Some(parent) = Path::new(&deleted_path)
            .parent()
            .and_then(|p| p.to_str())
            .filter(|p| !p.is_empty())
        {
            let new_id = Uuid::new_v4();
            let parent = format!("{parent}/");

            sqlx::query(&format!(
                "
                INSERT INTO {FILES_TABLE} (id, path, size, storage_id, is_uploaded)
                SELECT $1, $2, 0, $3, true
                WHERE
                    NOT EXISTS (
                        SELECT id
                        FROM {FILES_TABLE}
                        WHERE storage_id = $3
                          AND deleted_at IS NULL
                          AND path LIKE $2 || '%'
                    );
            "
            ))
            .bind(new_id)
            .bind(parent)
            .bind(storage_id)
            .execute(&mut *transaction)
            .await
            .map_err(|e| map_not_found(e, "some entity"))?;
        }

        transaction
            .commit()
            .await
            .map_err(|e| map_not_found(e, ""))?;

        Ok(())
    }

    pub async fn update_path(
        &self,
        old_path: &str,
        new_path: &str,
        storage_id: Uuid,
    ) -> SarcaResult<()> {
        self.rewrite_paths(old_path, new_path, storage_id, true).await
    }

    /// Rewrite paths for trashed rows only (used before restore-with-rename).
    pub async fn update_trashed_path(
        &self,
        old_path: &str,
        new_path: &str,
        storage_id: Uuid,
    ) -> SarcaResult<()> {
        self.rewrite_paths(old_path, new_path, storage_id, false).await
    }

    async fn rewrite_paths(
        &self,
        old_path: &str,
        new_path: &str,
        storage_id: Uuid,
        live_only: bool,
    ) -> SarcaResult<()> {
        let deleted_filter = if live_only {
            "AND deleted_at IS NULL"
        } else {
            "AND deleted_at IS NOT NULL"
        };
        let is_folder = old_path.ends_with('/');
        if is_folder {
            let old_prefix = old_path;
            let new_prefix = if new_path.ends_with('/') {
                new_path.to_string()
            } else {
                format!("{new_path}/")
            };
            let skip = old_prefix.len();
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE}
                SET path = $1 || SUBSTRING(path FROM {skip} + 1)
                WHERE storage_id = $2
                  {deleted_filter}
                  AND (path = $3 OR path LIKE $3 || '%')
                "
            ))
            .bind(&new_prefix)
            .bind(storage_id)
            .bind(old_prefix)
            .execute(self.db)
            .await
            .map_err(|_| SarcaError::Unknown)?;
        } else {
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE}
                SET path = $1
                WHERE storage_id = $2 {deleted_filter} AND path = $3
                "
            ))
            .bind(new_path)
            .bind(storage_id)
            .bind(old_path)
            .execute(self.db)
            .await
            .map_err(|_| SarcaError::Unknown)?;
        }
        Ok(())
    }

    pub async fn cleanup_stale_uploads(&self, older_than_minutes: i64) -> SarcaResult<u64> {
        let result = sqlx::query(&format!(
            "
            DELETE FROM {FILES_TABLE}
            WHERE is_uploaded = false
              AND deleted_at IS NULL
              AND path NOT LIKE '%/'
            "
        ))
        // Note: files table has no created_at; delete all unfinished uploads.
        // older_than_minutes reserved for future schema; currently cleans all stale rows.
        .execute(self.db)
        .await
        .map_err(|_| SarcaError::Unknown)?;

        let _ = older_than_minutes;
        Ok(result.rows_affected())
    }

    /// Directory listing for trashed items under `prefix` (without leading/trailing slashes).
    pub async fn list_trash(
        &self,
        storage_id: Uuid,
        prefix: &str,
    ) -> SarcaResult<Vec<FSElement>> {
        let query = {
            let adding_to_position = !prefix.is_empty() as usize + 1;
            let split_position = prefix.matches("/").count() + adding_to_position;
            let split_part = format!("SPLIT_PART(path, '/', {split_position})");
            let path_filter = if prefix.is_empty() {
                ""
            } else {
                "AND path LIKE $1 || '%'"
            };

            format!(
                "
                SELECT
                    DISTINCT {split_part} AS name,
                    $1 || {split_part} = path AS is_file,
                    CASE
                        WHEN $1 || {split_part} = path THEN size
                        ELSE (SELECT SUM(size) FROM {FILES_TABLE} WHERE deleted_at IS NOT NULL AND path LIKE $1 || {split_part} || '/' || '%')::BigInt
                    END AS size,
                    CASE
                        WHEN $1 || {split_part} = path THEN (thumb_telegram_file_id IS NOT NULL)
                        ELSE false
                    END AS has_thumb
                FROM {FILES_TABLE}
                WHERE storage_id = $2 {path_filter} AND is_uploaded AND deleted_at IS NOT NULL AND {split_part} <> '';
            "
            )
        };

        let prefix = if prefix.is_empty() {
            prefix.to_string()
        } else {
            format!("{prefix}/")
        };

        let fs_layer = sqlx::query_as::<_, DBFSElement>(&query)
            .bind(&prefix)
            .bind(storage_id)
            .fetch_all(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;
        let fs_layer = fs_layer
            .into_iter()
            .map(|el| {
                let path = format!("{prefix}{}", el.name);
                FSElement {
                    path,
                    name: el.name,
                    is_file: el.is_file,
                    size: el.size,
                    has_thumb: el.has_thumb,
                }
            })
            .collect();

        Ok(fs_layer)
    }

    #[allow(dead_code)]
    pub async fn get_trashed_file_by_path(
        &self,
        path: &str,
        storage_id: Uuid,
    ) -> SarcaResult<File> {
        sqlx::query_as(
            format!(
                "SELECT * FROM {FILES_TABLE} WHERE storage_id = $1 AND path = $2 AND deleted_at IS NOT NULL"
            )
            .as_str(),
        )
        .bind(storage_id)
        .bind(path)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(e, "file"))
    }

    /// Resolve trashed file ids matching a path or folder prefix.
    pub async fn list_trashed_ids(
        &self,
        storage_id: Uuid,
        path: &str,
    ) -> SarcaResult<Vec<Uuid>> {
        let is_folder = path.ends_with('/');
        let folder_prefix = if is_folder {
            path.to_string()
        } else {
            let probe = format!("{path}/");
            let has_folder: (bool,) = sqlx::query_as(&format!(
                "
                SELECT EXISTS(
                    SELECT 1 FROM {FILES_TABLE}
                    WHERE storage_id = $1
                      AND deleted_at IS NOT NULL
                      AND (path = $2 OR path LIKE $2 || '%')
                )
                "
            ))
            .bind(storage_id)
            .bind(&probe)
            .fetch_one(self.db)
            .await
            .map_err(|e| map_not_found(e, "file"))?;

            if has_folder.0 {
                probe
            } else {
                String::new()
            }
        };

        let rows: Vec<(Uuid,)> = if !folder_prefix.is_empty() {
            sqlx::query_as(&format!(
                "
                SELECT id FROM {FILES_TABLE}
                WHERE storage_id = $1
                  AND deleted_at IS NOT NULL
                  AND (path = $2 OR path LIKE $2 || '%')
                "
            ))
            .bind(storage_id)
            .bind(&folder_prefix)
            .fetch_all(self.db)
            .await
        } else {
            sqlx::query_as(&format!(
                "
                SELECT id FROM {FILES_TABLE}
                WHERE storage_id = $1 AND deleted_at IS NOT NULL AND path = $2
                "
            ))
            .bind(storage_id)
            .bind(path)
            .fetch_all(self.db)
            .await
        }
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    pub async fn list_all_trashed_ids(&self, storage_id: Uuid) -> SarcaResult<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as(&format!(
            "SELECT id FROM {FILES_TABLE} WHERE storage_id = $1 AND deleted_at IS NOT NULL"
        ))
        .bind(storage_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    pub async fn list_expired_trashed_ids(
        &self,
        older_than_days: i32,
    ) -> SarcaResult<Vec<(Uuid, Uuid)>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(&format!(
            "
            SELECT id, storage_id FROM {FILES_TABLE}
            WHERE deleted_at IS NOT NULL
              AND deleted_at < NOW() - ($1::text || ' days')::interval
            "
        ))
        .bind(older_than_days)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(rows)
    }

    pub async fn hard_delete_ids(&self, ids: &[Uuid]) -> SarcaResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        sqlx::query(&format!("DELETE FROM {FILES_TABLE} WHERE id = ANY($1)"))
            .bind(ids)
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;
        Ok(())
    }

    /// Clear deleted_at for a trashed path (file) or folder prefix.
    pub async fn restore(&self, path: &str, storage_id: Uuid) -> SarcaResult<()> {
        let is_folder = path.ends_with('/');
        let folder_prefix = if is_folder {
            path.to_string()
        } else {
            let probe = format!("{path}/");
            let has_folder: (bool,) = sqlx::query_as(&format!(
                "
                SELECT EXISTS(
                    SELECT 1 FROM {FILES_TABLE}
                    WHERE storage_id = $1
                      AND deleted_at IS NOT NULL
                      AND (path = $2 OR path LIKE $2 || '%')
                )
                "
            ))
            .bind(storage_id)
            .bind(&probe)
            .fetch_one(self.db)
            .await
            .map_err(|e| map_not_found(e, "file"))?;

            if has_folder.0 {
                probe
            } else {
                String::new()
            }
        };

        let affected = if !folder_prefix.is_empty() {
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE}
                SET deleted_at = NULL
                WHERE storage_id = $1
                  AND deleted_at IS NOT NULL
                  AND (path = $2 OR path LIKE $2 || '%')
                "
            ))
            .bind(storage_id)
            .bind(&folder_prefix)
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?
            .rows_affected()
        } else {
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE}
                SET deleted_at = NULL
                WHERE storage_id = $1 AND deleted_at IS NOT NULL AND path = $2
                "
            ))
            .bind(storage_id)
            .bind(path)
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?
            .rows_affected()
        };

        if affected == 0 {
            return Err(SarcaError::DoesNotExist("file".to_string()));
        }
        Ok(())
    }

    /// Ensure live parent folder markers exist for `path` (file or folder).
    pub async fn ensure_live_parent_folders(
        &self,
        path: &str,
        storage_id: Uuid,
    ) -> SarcaResult<()> {
        let trimmed = path.trim_end_matches('/');
        let Some(parent) = Path::new(trimmed)
            .parent()
            .and_then(|p| p.to_str())
            .filter(|p| !p.is_empty())
        else {
            return Ok(());
        };

        let mut acc = String::new();
        for part in parent.split('/') {
            if part.is_empty() {
                continue;
            }
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            let folder_path = format!("{acc}/");
            let exists: (bool,) = sqlx::query_as(&format!(
                "
                SELECT EXISTS(
                    SELECT 1 FROM {FILES_TABLE}
                    WHERE storage_id = $1 AND deleted_at IS NULL AND path = $2
                )
                "
            ))
            .bind(storage_id)
            .bind(&folder_path)
            .fetch_one(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;

            if !exists.0 {
                let id = Uuid::new_v4();
                sqlx::query(&format!(
                    "
                    INSERT INTO {FILES_TABLE} (id, path, size, storage_id, is_uploaded)
                    VALUES ($1, $2, 0, $3, true)
                    ON CONFLICT (path, storage_id) WHERE deleted_at IS NULL DO NOTHING
                    "
                ))
                .bind(id)
                .bind(&folder_path)
                .bind(storage_id)
                .execute(self.db)
                .await
                .map_err(|e| {
                    tracing::error!("{e}");
                    SarcaError::Unknown
                })?;
            }
        }
        Ok(())
    }

    pub async fn live_path_exists(&self, path: &str, storage_id: Uuid) -> SarcaResult<bool> {
        let row: (bool,) = sqlx::query_as(&format!(
            "
            SELECT EXISTS(
                SELECT 1 FROM {FILES_TABLE}
                WHERE storage_id = $1 AND deleted_at IS NULL AND path = $2
            )
            "
        ))
        .bind(storage_id)
        .bind(path)
        .fetch_one(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(row.0)
    }

    /// Next free live path for rename-on-conflict (`name (n).ext`).
    pub async fn next_available_live_path(
        &self,
        path: &str,
        storage_id: Uuid,
    ) -> SarcaResult<String> {
        if !self.live_path_exists(path, storage_id).await? {
            return Ok(path.to_string());
        }

        let (stem, suffix) = match path.rsplit_once('.') {
            Some((stem, ext)) if !stem.contains('/') || stem.rsplit('/').next().is_some() => {
                // Prefer last path segment for extension split
                if let Some((dir, name)) = path.rsplit_once('/') {
                    match name.rsplit_once('.') {
                        Some((n, e)) => (format!("{dir}/{n}"), format!(".{e}")),
                        None => (path.to_string(), String::new()),
                    }
                } else {
                    (stem.to_string(), format!(".{ext}"))
                }
            }
            _ => (path.to_string(), String::new()),
        };

        for i in 1..10_000 {
            let candidate = format!("{stem} ({i}){suffix}");
            if !self.live_path_exists(&candidate, storage_id).await? {
                return Ok(candidate);
            }
        }
        Err(SarcaError::Unknown)
    }

    pub async fn list_live_ids_at_path(
        &self,
        storage_id: Uuid,
        path: &str,
    ) -> SarcaResult<Vec<Uuid>> {
        let is_folder = path.ends_with('/');
        let folder_prefix = if is_folder {
            path.to_string()
        } else {
            let probe = format!("{path}/");
            let has_folder: (bool,) = sqlx::query_as(&format!(
                "
                SELECT EXISTS(
                    SELECT 1 FROM {FILES_TABLE}
                    WHERE storage_id = $1
                      AND deleted_at IS NULL
                      AND (path = $2 OR path LIKE $2 || '%')
                )
                "
            ))
            .bind(storage_id)
            .bind(&probe)
            .fetch_one(self.db)
            .await
            .map_err(|e| map_not_found(e, "file"))?;

            if has_folder.0 {
                probe
            } else {
                String::new()
            }
        };

        let rows: Vec<(Uuid,)> = if !folder_prefix.is_empty() {
            sqlx::query_as(&format!(
                "
                SELECT id FROM {FILES_TABLE}
                WHERE storage_id = $1
                  AND deleted_at IS NULL
                  AND (path = $2 OR path LIKE $2 || '%')
                "
            ))
            .bind(storage_id)
            .bind(&folder_prefix)
            .fetch_all(self.db)
            .await
        } else {
            sqlx::query_as(&format!(
                "
                SELECT id FROM {FILES_TABLE}
                WHERE storage_id = $1 AND deleted_at IS NULL AND path = $2
                "
            ))
            .bind(storage_id)
            .bind(path)
            .fetch_all(self.db)
            .await
        }
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}
