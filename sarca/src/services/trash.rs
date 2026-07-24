use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::{
        access::check_access,
        jwt_manager::AuthUser,
        telegram_api::bot_api::TelegramBotApi,
    },
    errors::{SarcaError, SarcaResult},
    models::{access::AccessType, files::FSElement},
    repositories::{
        access::AccessRepository, chunk_replicas::ChunkReplicasRepository, files::FilesRepository,
    },
    services::storage_workers_scheduler::StorageWorkersScheduler,
};

pub struct TrashService<'d> {
    files_repo: FilesRepository<'d>,
    access_repo: AccessRepository<'d>,
    db: &'d PgPool,
    base_url: &'d str,
    rate_limit: u8,
}

impl<'d> TrashService<'d> {
    pub fn new(db: &'d PgPool, base_url: &'d str, rate_limit: u8) -> Self {
        Self {
            files_repo: FilesRepository::new(db),
            access_repo: AccessRepository::new(db),
            db,
            base_url,
            rate_limit,
        }
    }

    pub async fn list(
        &self,
        storage_id: Uuid,
        path: &str,
        user: &AuthUser,
    ) -> SarcaResult<Vec<FSElement>> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::R).await?;
        self.files_repo.list_trash(storage_id, path).await
    }

    pub async fn restore(
        &self,
        storage_id: Uuid,
        path: &str,
        on_conflict: Option<&str>,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;
        if path.is_empty() {
            return Err(SarcaError::InvalidPath);
        }

        let restore_path = normalize_trash_path(path);
        let ids = self
            .files_repo
            .list_trashed_ids(storage_id, &restore_path)
            .await?;
        if ids.is_empty() {
            return Err(SarcaError::DoesNotExist("file".to_string()));
        }

        let canonical = canonical_trashed_path(&self.files_repo, storage_id, &restore_path).await?;

        let live_conflict = live_conflict_at(&self.files_repo, storage_id, &canonical).await?;

        if live_conflict {
            match on_conflict {
                None => return Err(SarcaError::TrashPathConflict),
                Some("replace") => {
                    let live_ids = self
                        .files_repo
                        .list_live_ids_at_path(storage_id, &canonical)
                        .await?;
                    self.purge_ids(&live_ids).await?;
                }
                Some("rename") => {
                    let new_path = self
                        .files_repo
                        .next_available_live_path(
                            canonical.trim_end_matches('/'),
                            storage_id,
                        )
                        .await?;
                    let new_path = if canonical.ends_with('/') {
                        format!("{}/", new_path.trim_end_matches('/'))
                    } else {
                        new_path
                    };
                    self.files_repo
                        .update_trashed_path(&canonical, &new_path, storage_id)
                        .await?;
                    self.files_repo
                        .ensure_live_parent_folders(&new_path, storage_id)
                        .await?;
                    return self.files_repo.restore(&new_path, storage_id).await;
                }
                Some(_) => return Err(SarcaError::InvalidPath),
            }
        }

        self.files_repo
            .ensure_live_parent_folders(&canonical, storage_id)
            .await?;
        self.files_repo.restore(&canonical, storage_id).await
    }

    pub async fn delete_forever(
        &self,
        storage_id: Uuid,
        path: &str,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;
        let path = normalize_trash_path(path);
        let ids = self.files_repo.list_trashed_ids(storage_id, &path).await?;
        if ids.is_empty() {
            return Err(SarcaError::DoesNotExist("file".to_string()));
        }
        self.purge_ids(&ids).await
    }

    pub async fn empty(&self, storage_id: Uuid, user: &AuthUser) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;
        let ids = self.files_repo.list_all_trashed_ids(storage_id).await?;
        self.purge_ids(&ids).await
    }

    pub async fn purge_ids(&self, ids: &[Uuid]) -> SarcaResult<()> {
        purge_file_ids(self.db, self.base_url, self.rate_limit, ids).await
    }
}

fn normalize_trash_path(path: &str) -> String {
    path.trim_start_matches('/').to_string()
}

async fn canonical_trashed_path(
    repo: &FilesRepository<'_>,
    storage_id: Uuid,
    path: &str,
) -> SarcaResult<String> {
    if path.ends_with('/') {
        return Ok(path.to_string());
    }
    let probe = format!("{path}/");
    let folder_ids = repo.list_trashed_ids(storage_id, &probe).await?;
    if !folder_ids.is_empty() {
        Ok(probe)
    } else {
        Ok(path.to_string())
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

/// Delete Telegram messages for file ids (best-effort), then hard-delete DB rows.
pub async fn purge_file_ids(
    db: &PgPool,
    base_url: &str,
    rate_limit: u8,
    ids: &[Uuid],
) -> SarcaResult<()> {
    if ids.is_empty() {
        return Ok(());
    }

    let replicas_repo = ChunkReplicasRepository::new(db);
    let files_repo = FilesRepository::new(db);
    let mut messages = replicas_repo.list_telegram_messages_for_files(ids).await?;
    messages.extend(files_repo.list_thumb_messages_for_files(ids).await?);

    if !messages.is_empty() {
        let scheduler = StorageWorkersScheduler::new(db, rate_limit);
        let api = TelegramBotApi::new(base_url, scheduler);
        for (chat_id, message_id, storage_id) in messages {
            if let Err(e) = api.delete_message(chat_id, message_id, storage_id).await {
                tracing::warn!(
                    "[TRASH] failed to delete Telegram message {message_id} in chat {chat_id}: {e}"
                );
            }
        }
    }

    files_repo.hard_delete_ids(ids).await
}
