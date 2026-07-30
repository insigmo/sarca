use std::path::PathBuf;

use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::{
    common::{
        access::check_access,
        channels::{
            ClientData,
            ClientMessage,
            ClientSender,
            StorageManagerData,
            StorageManagerMessage,
            UploadFileData,
            UploadProgressEvent,
        },
        jwt_manager::AuthUser,
    },
    errors::{SarcaError, SarcaResult},
    models::{
        access::AccessType,
        chunk_replicas::ChunkReplica,
        file_chunks::FileChunk,
        files::{FSElement, File, InFile, SearchFSElement},
    },
    repositories::{
        access::AccessRepository,
        chunk_replicas::ChunkReplicasRepository,
        files::FilesRepository,
        share_links::ShareLinksRepository,
        storage_workers::StorageWorkersRepository,
    },
    schemas::files::InFolderSchema,
    services::trash::purge_file_ids,
};

pub struct FilesService<'d> {
    repo: FilesRepository<'d>,
    replicas_repo: ChunkReplicasRepository<'d>,
    storage_workers_repo: StorageWorkersRepository<'d>,
    access_repo: AccessRepository<'d>,
    db: &'d SqlitePool,
    base_url: &'d str,
    rate_limit: u8,
    tx: ClientSender,
}

impl<'d> FilesService<'d> {
    pub fn new(db: &'d SqlitePool, tx: ClientSender, base_url: &'d str, rate_limit: u8) -> Self {
        let repo = FilesRepository::new(db);
        let replicas_repo = ChunkReplicasRepository::new(db);
        let storage_workers_repo = StorageWorkersRepository::new(db);
        let access_repo = AccessRepository::new(db);
        Self {
            repo,
            replicas_repo,
            storage_workers_repo,
            access_repo,
            db,
            base_url,
            rate_limit,
            tx,
        }
    }

    pub async fn create_folder(
        &self,
        in_schema: InFolderSchema,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        // 0. checking access
        check_access(&self.access_repo, user.id, in_schema.storage_id, &AccessType::W).await?;

        // 1. validation
        if !Self::validate_filepath(&in_schema.parent_path) {
            return Err(SarcaError::InvalidPath);
        }
        if in_schema.folder_name.contains('/') {
            return Err(SarcaError::InvalidFolderName);
        }

        // 2. constructing final values
        let path = if in_schema.parent_path.is_empty() {
            format!("{}/", in_schema.folder_name)
        } else {
            format!("{}/{}/", in_schema.parent_path, in_schema.folder_name)
        };
        let in_file = InFile::new(path, 0, in_schema.storage_id);

        // 3. saving to db
        self.repo.create_folder(in_file).await.map(|_| ())
    }

    pub async fn ensure_upload_allowed(
        &self,
        storage_id: Uuid,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;
        Self::check_storage_workers(self, storage_id).await
    }

    pub async fn upload_anyway_from_path_with_progress(
        &self,
        in_file: InFile,
        file_path: PathBuf,
        file_size: i64,
        user: &AuthUser,
        progress: Option<mpsc::Sender<UploadProgressEvent>>,
    ) -> SarcaResult<()> {
        // 0. checking access
        check_access(&self.access_repo, user.id, in_file.storage_id, &AccessType::W).await?;

        // 1. check whether storage got workers
        Self::check_storage_workers(self, in_file.storage_id).await?;

        // 2. saving file in db
        let file = self.repo.create_file_anyway(in_file).await?;

        self.upload_from_path(file, file_path, file_size, progress).await
    }

    async fn upload_from_path(
        &self,
        file: File,
        file_path: PathBuf,
        file_size: i64,
        progress: Option<mpsc::Sender<UploadProgressEvent>>,
    ) -> SarcaResult<()> {
        let (resp_tx, resp_rx) = oneshot::channel();

        let chunk_size =
            file.chunk_size_bytes.filter(|&n| n > 0).map(|n| n as usize).ok_or_else(|| {
                tracing::error!("upload missing chunk_size_bytes for file {}", file.id);
                SarcaError::Unknown
            })?;

        // Signal spool+DB ready before queuing Telegram so the client can pipeline
        // the next file's client→Sarca transfer. Storage Manager still serializes
        // Telegram uploads one at a time.
        if let Some(ref tx) = progress {
            let _ = tx.send(UploadProgressEvent::spooled(file_size.max(0) as u64)).await;
        }

        let message = ClientMessage {
            data: ClientData::UploadFile(UploadFileData {
                file_id: file.id,
                file_path,
                file_size,
                chunk_size,
                progress,
            }),
            tx: resp_tx,
        };

        tracing::debug!("sending task to manager");
        let _ = self.tx.send(message).await;

        // 3. waiting for a storage manager result
        // Prefer a clear error over panicking if the manager drops the oneshot
        // (e.g. process shutdown while this upload is queued).
        let message_back = match resp_rx.await {
            Ok(StorageManagerMessage {
                data: StorageManagerData::UploadFile(r),
            }) => r,
            Err(_) => {
                tracing::error!("storage manager dropped upload response for {}", file.id);
                Err(SarcaError::Unknown)
            },
        };
        if let Err(e) = message_back.and({
            tracing::debug!("file loaded successfully");

            // 4. setting file as uploaded
            self.repo.set_as_uploaded(file.id).await
        }) {
            tracing::error!("{e}");

            // fallback: hard-purge with refcount GC (may have partial Telegram uploads)
            let _ = purge_file_ids(self.db, self.base_url, self.rate_limit, &[file.id]).await;

            return Err(e);
        }

        Ok(())
    }

    async fn check_storage_workers(&self, storage_id: Uuid) -> SarcaResult<()> {
        if self.storage_workers_repo.storage_has_any(storage_id).await? {
            Ok(())
        } else {
            Err(SarcaError::StorageDoesNotHaveWorkers)
        }
    }

    pub async fn list_dir(
        self,
        storage_id: Uuid,
        path: &str,
        user: &AuthUser,
    ) -> SarcaResult<Vec<FSElement>> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::R).await?;

        self.repo.list_dir(storage_id, path).await
    }

    pub async fn info(
        &self,
        storage_id: Uuid,
        path: &str,
        user: &AuthUser,
    ) -> SarcaResult<crate::schemas::files::FileInfoSchema> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::R).await?;

        let path = path.trim_start_matches('/').to_owned();
        if path.contains("//") || path.contains("..") {
            return Err(SarcaError::InvalidPath);
        }

        // Prefer exact file row (not a folder marker).
        if !path.is_empty() && !path.ends_with('/') {
            if let Ok(file) = self.repo.get_file_by_path(&path, storage_id).await {
                let name = std::path::Path::new(&file.path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&file.path)
                    .to_owned();
                let content_type =
                    mime_guess::from_path(&name).first().map(|m| m.essence_str().to_owned());
                return Ok(crate::schemas::files::FileInfoSchema {
                    path: file.path.clone(),
                    name,
                    size: file.size,
                    is_file: true,
                    has_thumb: file.thumb_telegram_file_id.is_some(),
                    is_uploaded: file.is_uploaded,
                    content_type,
                    deleted_at: file.deleted_at,
                    added_at: Some(file.created_at),
                    created_at: file.source_created_at,
                    modified_at: file.source_mtime,
                    content_hash: file.content_hash,
                });
            }
        }

        // Folder: marker path ends with /
        let folder_path = if path.is_empty() {
            String::new()
        } else if path.ends_with('/') {
            path.clone()
        } else {
            format!("{path}/")
        };

        let marker = if folder_path.is_empty() {
            None
        } else {
            self.repo.get_file_by_path(&folder_path, storage_id).await.ok()
        };

        let size = if folder_path.is_empty() {
            self.repo.sum_uploaded_size_under(storage_id, "").await.unwrap_or(0)
        } else {
            self.repo.sum_uploaded_size_under(storage_id, &folder_path).await.unwrap_or(0)
        };

        let name = if folder_path.is_empty() {
            "/".to_owned()
        } else {
            folder_path.trim_end_matches('/').rsplit('/').next().unwrap_or(&folder_path).to_owned()
        };

        Ok(crate::schemas::files::FileInfoSchema {
            path: if folder_path.is_empty() {
                String::new()
            } else {
                folder_path.trim_end_matches('/').to_owned()
            },
            name,
            size,
            is_file: false,
            has_thumb: false,
            is_uploaded: marker.as_ref().is_none_or(|m| m.is_uploaded),
            content_type: None,
            deleted_at: marker.as_ref().and_then(|m| m.deleted_at),
            added_at: marker.as_ref().map(|m| m.created_at),
            created_at: marker.as_ref().and_then(|m| m.source_created_at),
            modified_at: marker.as_ref().and_then(|m| m.source_mtime),
            content_hash: marker.and_then(|m| m.content_hash),
        })
    }

    pub async fn search(
        self,
        storage_id: Uuid,
        path: &str,
        search_path: &str,
        user: &AuthUser,
    ) -> SarcaResult<Vec<SearchFSElement>> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::R).await?;

        self.repo.search(search_path, path, storage_id).await
    }

    pub async fn delete(&self, path: &str, storage_id: Uuid, user: &AuthUser) -> SarcaResult<()> {
        // 0. checking access
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;

        // 1. path validation
        if !Self::validate_path(path) {
            return Err(SarcaError::InvalidPath);
        }

        // 2. soft-delete only (Telegram untouched)
        let deleted_target = self.repo.delete(path, storage_id).await?;

        // 3. Shares are path-keyed — drop links that targeted this file/folder.
        let shares = ShareLinksRepository::new(self.db);
        if let Err(e) = shares.delete_for_target(storage_id, &deleted_target).await {
            tracing::warn!(
                "[SHARES] failed to delete links for trashed path {deleted_target}: {e}"
            );
        }

        Ok(())
    }

    pub async fn rename(
        &self,
        storage_id: Uuid,
        old_path: &str,
        new_path: &str,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;

        if !Self::validate_path(old_path) || !Self::validate_path(new_path) {
            return Err(SarcaError::InvalidPath);
        }
        if old_path.ends_with('/') != new_path.ends_with('/') {
            return Err(SarcaError::InvalidPath);
        }

        self.repo.update_path(old_path, new_path, storage_id).await?;
        self.rewrite_share_paths(storage_id, old_path, new_path).await;
        Ok(())
    }

    pub async fn move_to(
        &self,
        storage_id: Uuid,
        path: &str,
        destination_folder: &str,
        on_conflict: Option<&str>,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;

        if !Self::validate_path(path) || path.is_empty() {
            return Err(SarcaError::InvalidPath);
        }

        let dest = destination_folder.trim_end_matches('/');
        if !dest.is_empty() && !Self::validate_path(dest) {
            return Err(SarcaError::InvalidPath);
        }

        let source = self.repo.canonicalize_live_path(storage_id, path).await?;
        let mut new_path = Self::path_in_folder(&source, dest);
        if !Self::validate_path(&new_path) {
            return Err(SarcaError::InvalidPath);
        }

        // Same path → no-op
        if source == new_path {
            return Ok(());
        }

        // Moving a folder into itself / a descendant is invalid
        if source.ends_with('/') && new_path.starts_with(&source) {
            return Err(SarcaError::InvalidPath);
        }

        new_path = self.resolve_dest_conflict(storage_id, &source, new_path, on_conflict).await?;

        self.repo.ensure_live_parent_folders(&new_path, storage_id).await?;
        self.repo.update_path(&source, &new_path, storage_id).await?;
        self.rewrite_share_paths(storage_id, &source, &new_path).await;
        Ok(())
    }

    async fn rewrite_share_paths(&self, storage_id: Uuid, old_path: &str, new_path: &str) {
        let shares = ShareLinksRepository::new(self.db);
        if let Err(e) = shares.rewrite_paths(storage_id, old_path, new_path).await {
            tracing::warn!("[SHARES] failed to rewrite links {old_path} → {new_path}: {e}");
        }
    }

    pub async fn copy_to(
        &self,
        storage_id: Uuid,
        path: &str,
        destination_folder: &str,
        on_conflict: Option<&str>,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;

        if !Self::validate_path(path) || path.is_empty() {
            return Err(SarcaError::InvalidPath);
        }

        let dest = destination_folder.trim_end_matches('/');
        if !dest.is_empty() && !Self::validate_path(dest) {
            return Err(SarcaError::InvalidPath);
        }

        let source = self.repo.canonicalize_live_path(storage_id, path).await?;
        let mut dest_root = Self::path_in_folder(&source, dest);
        if !Self::validate_path(&dest_root) {
            return Err(SarcaError::InvalidPath);
        }

        dest_root = self.resolve_dest_conflict(storage_id, &source, dest_root, on_conflict).await?;

        self.repo.ensure_live_parent_folders(&dest_root, storage_id).await?;

        if source.ends_with('/') {
            let rows = self.repo.list_live_under(storage_id, &source).await?;
            if rows.is_empty() {
                return Err(SarcaError::DoesNotExist("file".to_string()));
            }
            let has_root_marker = rows.iter().any(|r| r.path == source);
            if !has_root_marker {
                let in_file = InFile::new(dest_root.clone(), 0, storage_id);
                self.repo.create_folder(in_file).await?;
            }
            for row in rows {
                let relative = row.path.strip_prefix(&source).unwrap_or(&row.path);
                let new_path = format!("{dest_root}{relative}");
                self.repo.ensure_live_parent_folders(&new_path, storage_id).await?;
                self.clone_one_row(&row, &new_path).await?;
            }
        } else {
            let file = self.repo.get_file_by_path(&source, storage_id).await?;
            self.clone_one_row(&file, &dest_root).await?;
        }

        Ok(())
    }

    /// Build a new path when renaming by basename only.
    pub fn rename_with_new_name(old_path: &str, new_name: &str) -> SarcaResult<String> {
        if new_name.is_empty() || new_name.contains('/') {
            return Err(SarcaError::InvalidFolderName);
        }

        let is_folder = old_path.ends_with('/');
        let trimmed = old_path.trim_end_matches('/');
        let parent = trimmed.rsplit_once('/').map_or("", |(p, _)| p);

        let new_path =
            if parent.is_empty() { new_name.to_owned() } else { format!("{parent}/{new_name}") };

        Ok(if is_folder { format!("{new_path}/") } else { new_path })
    }

    /////////////////////////////////////////////////////////////////////
    ////    Helpers
    //// /////////////////////////////////////////////////////////////////

    async fn resolve_dest_conflict(
        &self,
        storage_id: Uuid,
        source: &str,
        dest: String,
        on_conflict: Option<&str>,
    ) -> SarcaResult<String> {
        let conflict = live_conflict_at(&self.repo, storage_id, &dest).await?;
        if !conflict {
            return Ok(dest);
        }

        // Same path as the source (e.g. Ctrl+V into the same folder): cannot replace
        // a file with a copy of itself. Always keep both under a free name — same as
        // OS paste-duplicate — instead of 409 + a misleading "Replace" dialog.
        let self_overlap = dest == source
            || (source.ends_with('/') && dest.starts_with(source))
            || (dest.ends_with('/') && source.starts_with(&dest));
        if self_overlap {
            return self.repo.next_available_live_path(&dest, storage_id).await;
        }

        match on_conflict {
            None => Err(SarcaError::TrashPathConflict),
            Some("replace") => {
                let live_ids = self.repo.list_live_ids_at_path(storage_id, &dest).await?;
                purge_file_ids(self.db, self.base_url, self.rate_limit, &live_ids).await?;
                Ok(dest)
            },
            Some("rename") => self.repo.next_available_live_path(&dest, storage_id).await,
            Some(_) => Err(SarcaError::InvalidPath),
        }
    }

    async fn clone_one_row(&self, source: &File, dest_path: &str) -> SarcaResult<()> {
        let new_file = self.repo.insert_cloned_file(source, dest_path).await?;

        // Folder markers have no chunks.
        if dest_path.ends_with('/') || source.path.ends_with('/') {
            return Ok(());
        }

        let chunks = self.repo.list_chunks_of_file(source.id).await?;
        if chunks.is_empty() {
            return Ok(());
        }

        let mut chunk_id_map = std::collections::HashMap::new();
        let mut new_chunks = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            let new_id = Uuid::new_v4();
            chunk_id_map.insert(chunk.id, new_id);
            new_chunks.push(FileChunk::new(new_id, new_file.id, chunk.position));
        }
        self.repo.create_chunks_batch(new_chunks).await?;

        let replicas = self.replicas_repo.list_for_file(source.id).await?;
        if replicas.is_empty() {
            return Ok(());
        }

        let new_replicas: Vec<ChunkReplica> = replicas
            .into_iter()
            .filter_map(|r| {
                let new_chunk_id = *chunk_id_map.get(&r.chunk_id)?;
                Some(ChunkReplica {
                    id: Uuid::new_v4(),
                    chunk_id: new_chunk_id,
                    channel_id: r.channel_id,
                    telegram_file_id: r.telegram_file_id,
                    telegram_message_id: r.telegram_message_id,
                    status: r.status,
                })
            })
            .collect();
        self.replicas_repo.insert_batch(new_replicas).await?;
        Ok(())
    }

    fn path_in_folder(path: &str, dest_folder: &str) -> String {
        let is_folder = path.ends_with('/');
        let trimmed = path.trim_end_matches('/');
        let name = trimmed.rsplit('/').next().unwrap_or(trimmed);

        let new_path =
            if dest_folder.is_empty() { name.to_owned() } else { format!("{dest_folder}/{name}") };

        if is_folder { format!("{new_path}/") } else { new_path }
    }

    fn validate_filepath(path: &str) -> bool {
        Self::validate_path(path) && !path.ends_with('/')
    }

    fn validate_path(path: &str) -> bool {
        !path.starts_with('/') && !path.contains(r"//")
    }
}

async fn live_conflict_at(
    repo: &FilesRepository<'_>,
    storage_id: Uuid,
    path: &str,
) -> SarcaResult<bool> {
    if repo.live_path_exists(path, storage_id).await? {
        return Ok(true);
    }
    if !path.ends_with('/') && repo.live_path_exists(&format!("{path}/"), storage_id).await? {
        return Ok(true);
    }
    Ok(false)
}
