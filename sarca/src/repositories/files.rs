use std::{collections::HashMap, path::Path};

use chrono::{Duration as ChronoDuration, Utc};
use sqlx::{QueryBuilder, SqlitePool};
use uuid::Uuid;

use crate::{
    common::db::{errors::map_not_found, sql::push_uuid_list},
    errors::{SarcaError, SarcaResult},
    models::{
        file_chunks::{FileChunk, FileChunkWithReplica},
        files::{FSElement, File, InFile, SearchFSElement},
    },
};

pub const FILES_TABLE: &str = "files";
pub const CHUNKS_TABLE: &str = "file_chunks";

fn next_segment<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = if prefix.is_empty() { path } else { path.strip_prefix(prefix)? };
    rest.split('/').next().filter(|segment| !segment.is_empty())
}

fn pick_duplicate_path(path_with_stem: &str, suffix: &str, existing: &[String]) -> String {
    let base = format!("{path_with_stem}{suffix}");
    if !existing.iter().any(|path| path == &base) {
        return base;
    }

    if existing.len() == 1 {
        return format!("{path_with_stem} (1){suffix}");
    }

    let number_start = path_with_stem.len() + 2;
    let mut indices: Vec<i64> = existing
        .iter()
        .filter(|path| *path != &base)
        .filter_map(|path| {
            if path.len() <= number_start + suffix.len() + 1 {
                return None;
            }
            let number_end = path.len() - suffix.len() - 1;
            path[number_start..number_end].parse().ok().filter(|index| *index > 0)
        })
        .collect();
    indices.sort_unstable();

    let mut prev = 0i64;
    for index in indices {
        if prev != index - 1 {
            return format!("{path_with_stem} ({next}){suffix}", next = prev + 1);
        }
        prev = index;
    }
    format!("{path_with_stem} ({next}){suffix}", next = prev + 1)
}

/// Maps a `rewrite_paths` (rename/move) failure to a proper conflict error.
///
/// The rename target may already be occupied by another live (or, for
/// restore-with-rename, trashed) path, which trips the
/// `files_path_storage_id_alive_uidx` unique index. That must surface as a 409
/// conflict — same as `create_file_row`/`insert_cloned_file` do for the same
/// index — not as an opaque `SarcaError::Unknown` (500).
fn map_rewrite_path_error(e: sqlx::Error) -> SarcaError {
    match e {
        sqlx::Error::Database(dbe) if dbe.is_unique_violation() => {
            SarcaError::AlreadyExists("File with such name".to_string())
        },
        _ => {
            tracing::error!("{e}");
            SarcaError::Unknown
        },
    }
}

fn aggregate_dir_listing(rows: Vec<(String, i64, Option<String>)>, prefix: &str) -> Vec<FSElement> {
    #[derive(Default)]
    struct Acc {
        is_file: bool,
        file_size: i64,
        folder_size_sum: i64,
        has_thumb: bool,
    }

    let mut entries: HashMap<String, Acc> = HashMap::new();

    for (path, size, thumb_telegram_file_id) in rows {
        let Some(name) = next_segment(&path, prefix).map(str::to_owned) else {
            continue;
        };
        let entry_path = format!("{prefix}{name}");
        let acc = entries.entry(name.clone()).or_default();

        if entry_path == path {
            acc.is_file = true;
            acc.file_size = size;
            acc.has_thumb = thumb_telegram_file_id.is_some();
        } else {
            acc.folder_size_sum += size;
        }
    }

    entries
        .into_iter()
        .map(|(name, acc)| {
            let path = format!("{prefix}{name}");
            FSElement {
                path,
                name,
                is_file: acc.is_file,
                size: if acc.is_file { acc.file_size } else { acc.folder_size_sum },
                has_thumb: acc.has_thumb,
            }
        })
        .collect()
}

/// General repo for files and chunks since they share common logic
pub struct FilesRepository<'d> {
    db: &'d SqlitePool,
}

impl<'d> FilesRepository<'d> {
    pub fn new(db: &'d SqlitePool) -> Self {
        Self {
            db,
        }
    }

    pub async fn create_folder(&self, in_obj: InFile) -> SarcaResult<File> {
        self.create_file_row(in_obj, true).await
    }

    async fn create_file_row(&self, in_obj: InFile, is_uploaded: bool) -> SarcaResult<File> {
        let id = Uuid::new_v4();

        sqlx::query(
            format!(
                "
                INSERT INTO {FILES_TABLE} (id, path, size, storage_id, is_uploaded, \
                 chunk_size_bytes, source_created_at, source_mtime, content_hash)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9);
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
        .bind(in_obj.source_created_at)
        .bind(in_obj.source_mtime)
        .bind(&in_obj.content_hash)
        .execute(self.db)
        .await
        .map_err(|e| {
            match e {
                sqlx::Error::Database(dbe) if dbe.is_foreign_key_violation() => {
                    SarcaError::DoesNotExist("such storage".to_string())
                },
                sqlx::Error::Database(dbe) if dbe.is_unique_violation() => {
                    SarcaError::AlreadyExists("File with such name".to_string())
                },
                _ => {
                    tracing::error!("{e}");
                    SarcaError::Unknown
                },
            }
        })?;

        let mut storage = File::new(
            id,
            in_obj.path,
            in_obj.size,
            in_obj.storage_id,
            is_uploaded,
            in_obj.chunk_size_bytes,
        );
        storage.source_created_at = in_obj.source_created_at;
        storage.source_mtime = in_obj.source_mtime;
        storage.content_hash = in_obj.content_hash;
        Ok(storage)
    }

    /// Creates a file even if the given path already exists
    ///
    /// The dedup scan (find existing "name (N).ext" siblings) and the insert are two
    /// separate statements with no shared lock in between, so two concurrent calls for
    /// the same path/storage (e.g. an auto-upload client retry that overlaps the
    /// original attempt) can both compute the same free name and race on the
    /// `files_path_storage_id_alive_uidx` unique index. Rather than surface that as an
    /// opaque `SarcaError::Unknown`, retry the whole scan+insert cycle: the retry's scan
    /// observes the winner's just-committed row and picks the next free name.
    pub async fn create_file_anyway(&self, in_obj: InFile) -> SarcaResult<File> {
        // lol/kek/sdf.nj.dskf/sdkl.fdsklf/lol .kek.dsf
        let (path_with_stem, suffix) = {
            let mut splited_path: Vec<_> = in_obj.path.split('/').collect();
            let last = splited_path.last_mut().unwrap();
            let (stem, suffix) = last
                .split_once('.')
                .map(|(stem, suffix)| (stem, format!(".{suffix}")))
                .unwrap_or((last, String::new()));
            *last = stem;
            (splited_path.join("/"), suffix)
        };

        // Generous bound: only hit repeatedly by genuine concurrent collisions on the
        // exact same path, each retry narrowing the race window further.
        const MAX_ATTEMPTS: u32 = 20;
        let mut last_conflict_path = in_obj.path.clone();

        for attempt in 0..MAX_ATTEMPTS {
            let candidates: Vec<(String,)> = sqlx::query_as(
                format!(
                    r#"
                    SELECT path
                    FROM {FILES_TABLE}
                    WHERE storage_id = $1
                      AND deleted_at IS NULL
                      AND (path = $2 || $3 OR path LIKE $2 || ' (%)' || $3)
                    "#
                )
                .as_str(),
            )
            .bind(in_obj.storage_id)
            .bind(&path_with_stem)
            .bind(&suffix)
            .fetch_all(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;

            let base_path = format!("{path_with_stem}{suffix}");
            let matching: Vec<String> = candidates
                .into_iter()
                .map(|(path,)| path)
                .filter(|path| {
                    if path == &base_path {
                        return true;
                    }
                    if !path.starts_with(&path_with_stem) || !path.ends_with(&suffix) {
                        return false;
                    }
                    let middle = &path[path_with_stem.len()..path.len() - suffix.len()];
                    middle.starts_with(" (")
                        && middle.ends_with(')')
                        && middle[2..middle.len() - 1].chars().all(|ch| ch.is_ascii_digit())
                        && !middle[2..middle.len() - 1].is_empty()
                })
                .collect();

            let final_path = pick_duplicate_path(&path_with_stem, &suffix, &matching);
            let id = Uuid::new_v4();

            let result: Result<File, sqlx::Error> = sqlx::query_as(
                format!(
                    r#"
                    INSERT INTO {FILES_TABLE} (
                        path, storage_id, id, size, is_uploaded, chunk_size_bytes,
                        source_created_at, source_mtime, content_hash
                    )
                    VALUES ($1, $2, $3, $4, false, $5, $6, $7, $8)
                    RETURNING *
                    "#
                )
                .as_str(),
            )
            .bind(&final_path)
            .bind(in_obj.storage_id)
            .bind(id)
            .bind(in_obj.size)
            .bind(in_obj.chunk_size_bytes)
            .bind(in_obj.source_created_at)
            .bind(in_obj.source_mtime)
            .bind(&in_obj.content_hash)
            .fetch_one(self.db)
            .await;

            match result {
                Ok(file) => return Ok(file),
                Err(sqlx::Error::Database(dbe)) if dbe.is_unique_violation() => {
                    last_conflict_path = final_path;
                    tracing::debug!(
                        "create_file_anyway: lost race for {last_conflict_path} (attempt \
                         {attempt}), retrying"
                    );
                },
                Err(sqlx::Error::Database(dbe)) if dbe.is_foreign_key_violation() => {
                    return Err(SarcaError::DoesNotExist("such storage".to_string()));
                },
                Err(e) => {
                    tracing::error!("{e}");
                    return Err(SarcaError::Unknown);
                },
            }
        }

        tracing::error!(
            "create_file_anyway: giving up after {MAX_ATTEMPTS} attempts racing on \
             {last_conflict_path}"
        );
        Err(SarcaError::AlreadyExists(last_conflict_path))
    }

    pub async fn create_chunks_batch(&self, chunks: Vec<FileChunk>) -> SarcaResult<()> {
        // Empty files upload no chunks at all; `push_values` on an empty list would
        // build an INSERT without a VALUES clause and fail.
        if chunks.is_empty() {
            return Ok(());
        }

        QueryBuilder::new(format!("INSERT INTO {CHUNKS_TABLE} (id, file_id, position) ").as_str())
            .push_values(chunks, |mut q, chunk| {
                q.push_bind(chunk.id).push_bind(chunk.file_id).push_bind(chunk.position);
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
    pub async fn list_dir(&self, storage_id: Uuid, prefix: &str) -> SarcaResult<Vec<FSElement>> {
        let prefix = if prefix.is_empty() { prefix.to_string() } else { format!("{prefix}/") };
        let path_filter =
            if prefix.is_empty() { String::new() } else { "AND path LIKE $1 || '%'".to_string() };

        let rows: Vec<(String, i64, Option<String>)> = sqlx::query_as(&format!(
            "
            SELECT path, size, thumb_telegram_file_id
            FROM {FILES_TABLE}
            WHERE storage_id = $2 {path_filter} AND is_uploaded AND deleted_at IS NULL
            "
        ))
        .bind(&prefix)
        .bind(storage_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(aggregate_dir_listing(rows, &prefix))
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
                WHERE storage_id = $1 AND deleted_at IS NULL AND lower(path) LIKE lower($2) || '%' || lower($3) || '%'
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
                "SELECT * FROM {FILES_TABLE} WHERE storage_id = $1 AND path = $2 AND deleted_at \
                 IS NULL"
            )
            .as_str(),
        )
        .bind(storage_id)
        .bind(path)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(&e, "file"))
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
                SELECT COALESCE(SUM(size), 0)
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
            .map_err(|e| map_not_found(&e, "file"))
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

    pub async fn set_preview(
        &self,
        file_id: Uuid,
        preview_telegram_file_id: &str,
        preview_telegram_message_id: i64,
    ) -> SarcaResult<()> {
        sqlx::query(
            format!(
                "
                UPDATE {FILES_TABLE}
                SET preview_telegram_file_id = $2,
                    preview_telegram_message_id = $3
                WHERE id = $1
                "
            )
            .as_str(),
        )
        .bind(file_id)
        .bind(preview_telegram_file_id)
        .bind(preview_telegram_message_id)
        .execute(self.db)
        .await
        .map_err(|_| SarcaError::Unknown)
        .map(|_| ())
    }

    /// `(chat_id, message_id, storage_id)` for derived (thumbnail + preview) Telegram
    /// messages of the given files.
    pub async fn list_derived_messages_for_files(
        &self,
        file_ids: &[Uuid],
    ) -> SarcaResult<Vec<(i64, i64, Uuid)>> {
        if file_ids.is_empty() {
            return Ok(vec![]);
        }
        // Derived documents go to the primary channel at upload time; try all storage
        // channels so purge still works if primary later rotated.
        let mut builder = QueryBuilder::new(
            format!(
                "
                SELECT DISTINCT sc.chat_id, m.message_id, f.storage_id
                FROM {FILES_TABLE} f
                JOIN storage_channels sc ON sc.storage_id = f.storage_id
                JOIN (
                    SELECT id, thumb_telegram_message_id AS message_id FROM {FILES_TABLE}
                    UNION ALL
                    SELECT id, preview_telegram_message_id AS message_id FROM {FILES_TABLE}
                ) m ON m.id = f.id
                WHERE m.message_id IS NOT NULL
                  AND f.id IN ("
            )
            .as_str(),
        );
        push_uuid_list(&mut builder, file_ids);
        builder.push(")");

        let rows: Vec<(i64, Option<i64>, Uuid)> =
            builder.build_query_as().fetch_all(self.db).await.map_err(|e| {
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

    /// `(chat_id, message_id)` for derived (thumbnail + preview) Telegram messages of all
    /// files in a storage.
    pub async fn list_derived_messages_for_storage(
        &self,
        storage_id: Uuid,
    ) -> SarcaResult<Vec<(i64, i64)>> {
        let rows: Vec<(i64, Option<i64>)> = sqlx::query_as(
            format!(
                "
                SELECT DISTINCT sc.chat_id, m.message_id
                FROM {FILES_TABLE} f
                JOIN storage_channels sc ON sc.storage_id = f.storage_id
                JOIN (
                    SELECT id, thumb_telegram_message_id AS message_id FROM {FILES_TABLE}
                    UNION ALL
                    SELECT id, preview_telegram_message_id AS message_id FROM {FILES_TABLE}
                ) m ON m.id = f.id
                WHERE f.storage_id = $1
                  AND m.message_id IS NOT NULL
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
        })?;

        Ok(rows
            .into_iter()
            .filter_map(|(chat_id, message_id)| message_id.map(|mid| (chat_id, mid)))
            .collect())
    }

    /// True if any remaining file (live or trashed) still uses this message as its
    /// thumbnail or preview on a channel with `chat_id`.
    pub async fn derived_message_still_referenced(
        &self,
        chat_id: i64,
        message_id: i64,
    ) -> SarcaResult<bool> {
        let row: (bool,) = sqlx::query_as(
            format!(
                "
                SELECT EXISTS(
                    SELECT 1
                    FROM {FILES_TABLE} f
                    JOIN storage_channels sc ON sc.storage_id = f.storage_id
                    WHERE sc.chat_id = $1
                      AND $2 IN (f.thumb_telegram_message_id, f.preview_telegram_message_id)
                )
                "
            )
            .as_str(),
        )
        .bind(chat_id)
        .bind(message_id)
        .fetch_one(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(row.0)
    }

    pub async fn list_chunks_of_file(&self, file_id: Uuid) -> SarcaResult<Vec<FileChunk>> {
        sqlx::query_as(format!("SELECT * FROM {CHUNKS_TABLE} WHERE file_id = $1").as_str())
            .bind(file_id)
            .fetch_all(self.db)
            .await
            .map_err(|e| map_not_found(&e, "file chunks"))
    }

    pub async fn set_as_uploaded(&self, file_id: Uuid) -> SarcaResult<()> {
        sqlx::query(format!("UPDATE {FILES_TABLE} SET is_uploaded = true WHERE id = $1").as_str())
            .bind(file_id)
            .execute(self.db)
            .await
            .map_err(|_| SarcaError::Unknown)
            .map(|_| ())
    }

    /// Soft-delete live file(s) under `path`. Returns the canonical deleted target
    /// (folders end with `/`) for callers that need to clean up path-keyed metadata.
    pub async fn delete(&self, path: &str, storage_id: Uuid) -> SarcaResult<String> {
        let mut transaction = self.db.begin().await.map_err(|e| map_not_found(&e, ""))?;

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
            .map_err(|e| map_not_found(&e, "file"))?;

            if has_folder.0 { probe } else { String::new() }
        };

        let affected = if folder_prefix.is_empty() {
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE}
                SET deleted_at = datetime('now')
                WHERE storage_id = $1 AND deleted_at IS NULL AND path = $2;
                "
            ))
            .bind(storage_id)
            .bind(path)
            .execute(&mut *transaction)
            .await
            .map_err(|e| map_not_found(&e, "file"))?
            .rows_affected()
        } else {
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE}
                SET deleted_at = datetime('now')
                WHERE storage_id = $1
                  AND deleted_at IS NULL
                  AND (path = $2 OR path LIKE $2 || '%');
                "
            ))
            .bind(storage_id)
            .bind(&folder_prefix)
            .execute(&mut *transaction)
            .await
            .map_err(|e| map_not_found(&e, "file"))?
            .rows_affected()
        };

        if affected == 0 {
            return Err(SarcaError::DoesNotExist("file".to_string()));
        }

        let deleted_target =
            if folder_prefix.is_empty() { path.to_string() } else { folder_prefix.clone() };

        // Recreate parent folder marker only for non-root parents that became empty.
        let deleted_path = if folder_prefix.is_empty() {
            path.to_string()
        } else {
            folder_prefix.trim_end_matches('/').to_string()
        };
        if let Some(parent) =
            Path::new(&deleted_path).parent().and_then(|p| p.to_str()).filter(|p| !p.is_empty())
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
            .map_err(|e| map_not_found(&e, "some entity"))?;
        }

        transaction.commit().await.map_err(|e| map_not_found(&e, ""))?;

        Ok(deleted_target)
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
        let deleted_filter =
            if live_only { "AND deleted_at IS NULL" } else { "AND deleted_at IS NOT NULL" };
        let is_folder = old_path.ends_with('/');
        if is_folder {
            let old_prefix = old_path;
            let new_prefix =
                if new_path.ends_with('/') { new_path.to_string() } else { format!("{new_path}/") };
            let skip = old_prefix.len();
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE}
                SET path = $1 || substr(path, {skip} + 1)
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
            .map_err(map_rewrite_path_error)?;
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
            .map_err(map_rewrite_path_error)?;
        }
        Ok(())
    }

    /// Directory listing for trashed items under `prefix` (without leading/trailing slashes).
    pub async fn list_trash(&self, storage_id: Uuid, prefix: &str) -> SarcaResult<Vec<FSElement>> {
        let prefix = if prefix.is_empty() { prefix.to_string() } else { format!("{prefix}/") };
        let path_filter =
            if prefix.is_empty() { String::new() } else { "AND path LIKE $1 || '%'".to_string() };

        let rows: Vec<(String, i64, Option<String>)> = sqlx::query_as(&format!(
            "
            SELECT path, size, thumb_telegram_file_id
            FROM {FILES_TABLE}
            WHERE storage_id = $2 {path_filter} AND is_uploaded AND deleted_at IS NOT NULL
            "
        ))
        .bind(&prefix)
        .bind(storage_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(aggregate_dir_listing(rows, &prefix))
    }

    /// Resolve trashed file ids matching a path or folder prefix.
    pub async fn list_trashed_ids(&self, storage_id: Uuid, path: &str) -> SarcaResult<Vec<Uuid>> {
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
            .map_err(|e| map_not_found(&e, "file"))?;

            if has_folder.0 { probe } else { String::new() }
        };

        let rows: Vec<(Uuid,)> = if folder_prefix.is_empty() {
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
        } else {
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
        let cutoff = Utc::now() - ChronoDuration::days(i64::from(older_than_days));
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(&format!(
            "
            SELECT id, storage_id FROM {FILES_TABLE}
            WHERE deleted_at IS NOT NULL
              AND deleted_at < $1
            "
        ))
        .bind(cutoff)
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
        let mut builder = QueryBuilder::new(format!("DELETE FROM {FILES_TABLE} WHERE id IN ("));
        push_uuid_list(&mut builder, ids);
        builder.push(")");
        builder.build().execute(self.db).await.map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(())
    }

    /// Clear `deleted_at` for a trashed path (file) or folder prefix.
    ///
    /// Skips trashed rows whose path is already occupied by a live row (e.g. a
    /// parent folder marker recreated earlier) so restore cannot hit the alive
    /// unique index.
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
            .map_err(|e| map_not_found(&e, "file"))?;

            if has_folder.0 { probe } else { String::new() }
        };

        let affected = if folder_prefix.is_empty() {
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE} AS t
                SET deleted_at = NULL
                WHERE t.storage_id = $1
                  AND t.deleted_at IS NOT NULL
                  AND t.path = $2
                  AND NOT EXISTS (
                      SELECT 1 FROM {FILES_TABLE} AS live
                      WHERE live.storage_id = t.storage_id
                        AND live.path = t.path
                        AND live.deleted_at IS NULL
                  )
                "
            ))
            .bind(storage_id)
            .bind(path)
            .execute(self.db)
            .await
            .map_err(|e| {
                match e {
                    sqlx::Error::Database(ref dbe) if dbe.is_unique_violation() => {
                        SarcaError::TrashPathConflict
                    },
                    _ => {
                        tracing::error!("{e}");
                        SarcaError::Unknown
                    },
                }
            })?
            .rows_affected()
        } else {
            sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE} AS t
                SET deleted_at = NULL
                WHERE t.storage_id = $1
                  AND t.deleted_at IS NOT NULL
                  AND (t.path = $2 OR t.path LIKE $2 || '%')
                  AND NOT EXISTS (
                      SELECT 1 FROM {FILES_TABLE} AS live
                      WHERE live.storage_id = t.storage_id
                        AND live.path = t.path
                        AND live.deleted_at IS NULL
                  )
                "
            ))
            .bind(storage_id)
            .bind(&folder_prefix)
            .execute(self.db)
            .await
            .map_err(|e| {
                match e {
                    sqlx::Error::Database(ref dbe) if dbe.is_unique_violation() => {
                        SarcaError::TrashPathConflict
                    },
                    _ => {
                        tracing::error!("{e}");
                        SarcaError::Unknown
                    },
                }
            })?
            .rows_affected()
        };

        if affected == 0 {
            // Trashed rows may remain but be blocked by live twins at the same path.
            let blocked = self.list_trashed_ids(storage_id, path).await?;
            if !blocked.is_empty() {
                return Err(SarcaError::TrashPathConflict);
            }
            // Idempotent: already fully restored.
            if self.live_path_exists(path, storage_id).await?
                || (!path.ends_with('/')
                    && self.live_path_exists(&format!("{path}/"), storage_id).await?)
            {
                return Ok(());
            }
            return Err(SarcaError::DoesNotExist("file".to_string()));
        }
        Ok(())
    }

    /// Ensure live parent folder markers exist for `path` (file or folder).
    ///
    /// For each missing segment, prefer undeleting an existing trashed folder
    /// marker at that path; only insert a new empty folder when none exists.
    pub async fn ensure_live_parent_folders(
        &self,
        path: &str,
        storage_id: Uuid,
    ) -> SarcaResult<()> {
        let trimmed = path.trim_end_matches('/');
        let Some(parent) =
            Path::new(trimmed).parent().and_then(|p| p.to_str()).filter(|p| !p.is_empty())
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
            let live_exists: (bool,) = sqlx::query_as(&format!(
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

            if live_exists.0 {
                continue;
            }

            // Prefer restoring a trashed folder marker (exact path only — do not
            // pull nested trash contents back with the parent).
            let restored = sqlx::query(&format!(
                "
                UPDATE {FILES_TABLE}
                SET deleted_at = NULL
                WHERE id = (
                    SELECT id FROM {FILES_TABLE}
                    WHERE storage_id = $1
                      AND path = $2
                      AND deleted_at IS NOT NULL
                    ORDER BY deleted_at DESC
                    LIMIT 1
                )
                "
            ))
            .bind(storage_id)
            .bind(&folder_path)
            .execute(self.db)
            .await
            .map_err(|e| {
                match e {
                    sqlx::Error::Database(ref dbe) if dbe.is_unique_violation() => {
                        SarcaError::TrashPathConflict
                    },
                    _ => {
                        tracing::error!("{e}");
                        SarcaError::Unknown
                    },
                }
            })?
            .rows_affected();

            if restored > 0 {
                continue;
            }

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

    /// True if a live file `path` or folder `path/` occupies this basename.
    pub async fn live_basename_taken(&self, path: &str, storage_id: Uuid) -> SarcaResult<bool> {
        let file_path = path.trim_end_matches('/');
        if self.live_path_exists(file_path, storage_id).await? {
            return Ok(true);
        }
        let folder_path = format!("{file_path}/");
        self.live_path_exists(&folder_path, storage_id).await
    }

    /// Next free live path for rename-on-conflict (`name (n).ext` or `name (n)/`).
    ///
    /// Folder paths must end with `/`. If a non-slash path is passed but only the
    /// folder form is taken, treat it as a folder rename so we never return a
    /// "free" file path that collides with `path/` on restore.
    pub async fn next_available_live_path(
        &self,
        path: &str,
        storage_id: Uuid,
    ) -> SarcaResult<String> {
        let as_folder = path.ends_with('/')
            || (!path.is_empty()
                && self
                    .live_path_exists(&format!("{}/", path.trim_end_matches('/')), storage_id)
                    .await?
                && !self.live_path_exists(path.trim_end_matches('/'), storage_id).await?);

        let normalized = if as_folder {
            let stem = path.trim_end_matches('/');
            format!("{stem}/")
        } else {
            path.trim_end_matches('/').to_string()
        };

        if !self.live_basename_taken(&normalized, storage_id).await? {
            return Ok(normalized);
        }

        if as_folder {
            let stem = normalized.trim_end_matches('/');
            for i in 1..10_000 {
                let candidate = format!("{stem} ({i})/");
                if !self.live_basename_taken(&candidate, storage_id).await? {
                    return Ok(candidate);
                }
            }
            return Err(SarcaError::Unknown);
        }

        let (stem, suffix) = if let Some((dir, name)) = normalized.rsplit_once('/') {
            match name.rsplit_once('.') {
                Some((n, e)) => (format!("{dir}/{n}"), format!(".{e}")),
                None => (normalized.clone(), String::new()),
            }
        } else {
            match normalized.rsplit_once('.') {
                Some((stem, ext)) => (stem.to_string(), format!(".{ext}")),
                None => (normalized.clone(), String::new()),
            }
        };

        for i in 1..10_000 {
            let candidate = format!("{stem} ({i}){suffix}");
            if !self.live_basename_taken(&candidate, storage_id).await? {
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
            .map_err(|e| map_not_found(&e, "file"))?;

            if has_folder.0 { probe } else { String::new() }
        };

        let rows: Vec<(Uuid,)> = if folder_prefix.is_empty() {
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
        } else {
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
        }
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Resolve a live path to its canonical form (add trailing `/` when it is a folder).
    pub async fn canonicalize_live_path(
        &self,
        storage_id: Uuid,
        path: &str,
    ) -> SarcaResult<String> {
        if path.ends_with('/') {
            if self.live_path_exists(path, storage_id).await? {
                return Ok(path.to_string());
            }
            return Err(SarcaError::DoesNotExist("file".to_string()));
        }

        if self.live_path_exists(path, storage_id).await? {
            return Ok(path.to_string());
        }

        let folder = format!("{path}/");
        if self.live_path_exists(&folder, storage_id).await?
            || !self.list_live_ids_at_path(storage_id, &folder).await?.is_empty()
        {
            return Ok(folder);
        }

        Err(SarcaError::DoesNotExist("file".to_string()))
    }

    /// All live rows under a folder prefix (including the folder marker and nested folders).
    pub async fn list_live_under(
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
                  AND deleted_at IS NULL
                  AND (path = $2 OR path LIKE $2 || '%')
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

    /// Insert a live file row copying size/flags/thumb/preview/chunk size from `source`.
    pub async fn insert_cloned_file(&self, source: &File, dest_path: &str) -> SarcaResult<File> {
        let id = Uuid::new_v4();
        sqlx::query_as(
            format!(
                "
                INSERT INTO {FILES_TABLE} (
                    id, path, size, storage_id, is_uploaded,
                    thumb_telegram_file_id, thumb_telegram_message_id, chunk_size_bytes,
                    source_created_at, source_mtime,
                    preview_telegram_file_id, preview_telegram_message_id
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                RETURNING *
                "
            )
            .as_str(),
        )
        .bind(id)
        .bind(dest_path)
        .bind(source.size)
        .bind(source.storage_id)
        .bind(source.is_uploaded)
        .bind(&source.thumb_telegram_file_id)
        .bind(source.thumb_telegram_message_id)
        .bind(source.chunk_size_bytes)
        .bind(source.source_created_at)
        .bind(source.source_mtime)
        .bind(&source.preview_telegram_file_id)
        .bind(source.preview_telegram_message_id)
        .fetch_one(self.db)
        .await
        .map_err(|e| {
            match e {
                sqlx::Error::Database(dbe) if dbe.is_unique_violation() => {
                    SarcaError::AlreadyExists("File with such name".to_string())
                },
                _ => {
                    tracing::error!("{e}");
                    SarcaError::Unknown
                },
            }
        })
    }

    /// Ids of unfinished live uploads (for refcount-aware hard purge).
    pub async fn list_stale_upload_ids(&self) -> SarcaResult<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as(&format!(
            "
            SELECT id FROM {FILES_TABLE}
            WHERE is_uploaded = false
              AND deleted_at IS NULL
              AND path NOT LIKE '%/'
            "
        ))
        .fetch_all(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sarca.sqlite");
        let pool = crate::common::db::pool::get_pool(
            path.to_str().unwrap(),
            8,
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
        crate::startup::init_db(&pool).await;
        // Keep the tempdir (and its backing file) alive for the pool's lifetime.
        std::mem::forget(dir);
        pool
    }

    async fn insert_storage(db: &SqlitePool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO storages (id, name, primary_position) VALUES ($1, $2, 1)")
            .bind(id)
            .bind("test storage")
            .execute(db)
            .await
            .unwrap();
        id
    }

    /// Two auto-upload retries racing to create the same path (e.g. a client retry that
    /// overlaps the original attempt before either finishes) must not surface the
    /// unique-constraint collision as an opaque `SarcaError::Unknown` (500) — the loser
    /// should retry and land on a "(1)"-suffixed name, same as it would if the collision
    /// were observed sequentially.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn create_file_anyway_survives_concurrent_same_path_race() {
        let db = test_pool().await;
        let storage_id = insert_storage(&db).await;

        let mut handles = Vec::new();
        for _ in 0..8 {
            let db = db.clone();
            handles.push(tokio::spawn(async move {
                let repo = FilesRepository::new(&db);
                let in_file = InFile::new("photo.jpg".to_owned(), 100, storage_id);
                repo.create_file_anyway(in_file).await
            }));
        }

        let mut paths = std::collections::HashSet::new();
        for h in handles {
            let file = h.await.unwrap().expect("concurrent upload of same path must not fail");
            assert!(paths.insert(file.path.clone()), "duplicate path assigned: {}", file.path);
        }
        assert_eq!(paths.len(), 8, "every concurrent upload must land on a distinct path");
    }
}

#[cfg(test)]
mod rename_conflict_tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sarca.sqlite");
        let pool = crate::common::db::pool::get_pool(
            path.to_str().unwrap(),
            8,
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
        crate::startup::init_db(&pool).await;
        // Keep the tempdir (and its backing file) alive for the pool's lifetime.
        std::mem::forget(dir);
        pool
    }

    async fn insert_storage(db: &SqlitePool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO storages (id, name, primary_position) VALUES ($1, $2, 1)")
            .bind(id)
            .bind("test storage")
            .execute(db)
            .await
            .unwrap();
        id
    }

    /// Renaming a file onto a path that is already occupied by another live file
    /// hits the `files_path_storage_id_alive_uidx` unique index. `update_path` /
    /// `rewrite_paths` must surface this as a proper conflict error (matching
    /// `create_file_row`/`insert_cloned_file`'s handling of the same index), not
    /// an opaque `SarcaError::Unknown` (500).
    #[tokio::test]
    async fn update_path_reports_conflict_not_unknown() {
        let db = test_pool().await;
        let storage_id = insert_storage(&db).await;
        let repo = FilesRepository::new(&db);

        repo.create_folder(InFile::new("a.txt".to_owned(), 10, storage_id)).await.unwrap();
        repo.create_folder(InFile::new("b.txt".to_owned(), 20, storage_id)).await.unwrap();

        let err = repo
            .update_path("a.txt", "b.txt", storage_id)
            .await
            .expect_err("renaming onto an existing live path must fail");

        assert!(
            matches!(err, SarcaError::AlreadyExists(_) | SarcaError::TrashPathConflict),
            "expected a conflict error, got {err:?}"
        );
    }
}
