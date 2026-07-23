use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::{
        channels::{UploadFileData},
        telegram_api::bot_api::TelegramBotApi,
        types::ChatId,
    },
    errors::SarcaResult,
    models::file_chunks::FileChunk,
    repositories::{files::FilesRepository, storages::StoragesRepository},
    services::thumbnails,
};

use super::storage_workers_scheduler::StorageWorkersScheduler;

pub struct StorageManagerService<'d> {
    storages_repo: StoragesRepository<'d>,
    files_repo: FilesRepository<'d>,
    telegram_baseurl: &'d str,
    db: &'d PgPool,
    chunk_size: usize,
    rate_limit: u8,
}

impl<'d> StorageManagerService<'d> {
    pub fn new(db: &'d PgPool, telegram_baseurl: &'d str, rate_limit: u8, chunk_size: usize) -> Self {
        let files_repo = FilesRepository::new(db);
        let storages_repo = StoragesRepository::new(db);
        Self {
            storages_repo,
            files_repo,
            chunk_size,
            telegram_baseurl,
            db,
            rate_limit,
        }
    }

    pub async fn upload(&self, data: UploadFileData) -> SarcaResult<()> {
        let storage = self.storages_repo.get_by_file_id(data.file_id).await?;

        let mut position: usize = 0;
        let mut chunks: Vec<FileChunk> = Vec::new();

        let mut offset: u64 = 0;
        let total: u64 = data.file_size.max(0) as u64;

        while offset < total {
            let len = std::cmp::min(self.chunk_size as u64, total - offset);
            let chunk = self
                .upload_chunk_from_file(
                    storage.id,
                    storage.chat_id,
                    data.file_id,
                    position,
                    &data.file_path,
                    offset,
                    len,
                )
                .await?;
            chunks.push(chunk);
            offset += len;
            position += 1;
        }

        let result = self.files_repo.create_chunks_batch(chunks).await;

        if result.is_ok() {
            if let Err(e) = self
                .maybe_upload_thumb(data.file_id, storage.id, storage.chat_id, &data.file_path)
                .await
            {
                tracing::warn!("thumbnail upload failed for {}: {e}", data.file_id);
            }
        }

        let _ = tokio::fs::remove_file(&data.file_path).await;
        result
    }

    async fn maybe_upload_thumb(
        &self,
        file_id: Uuid,
        storage_id: Uuid,
        chat_id: ChatId,
        file_path: &std::path::Path,
    ) -> SarcaResult<()> {
        let file = self.files_repo.get_by_id(file_id).await?;

        let jpeg = match thumbnails::generate(file_path, &file.path).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(()),
            Err(e) => {
                tracing::warn!("thumbnail generation failed: {e}");
                return Ok(());
            }
        };

        let scheduler = StorageWorkersScheduler::new(self.db, self.rate_limit);
        let document = TelegramBotApi::new(self.telegram_baseurl, scheduler)
            .upload(&jpeg, chat_id, storage_id)
            .await?;

        self.files_repo
            .set_thumb(file_id, &document.file_id)
            .await?;

        tracing::debug!(
            "uploaded thumbnail for file {} as telegram_file_id {}",
            file_id,
            document.file_id
        );

        Ok(())
    }

    async fn upload_chunk_from_file(
        &self,
        storage_id: Uuid,
        chat_id: ChatId,
        file_id: Uuid,
        position: usize,
        file_path: &std::path::Path,
        offset: u64,
        len: u64,
    ) -> SarcaResult<FileChunk> {
        let scheduler = StorageWorkersScheduler::new(self.db, self.rate_limit);

        let document = TelegramBotApi::new(self.telegram_baseurl, scheduler)
            .upload_file_part(file_path, offset, len, chat_id, storage_id)
            .await?;

        tracing::debug!(
            "[TELEGRAM API] uploaded chunk with file_id \"{}\" and position \"{}\"",
            document.file_id,
            position
        );

        Ok(FileChunk::new(
            Uuid::new_v4(),
            file_id,
            document.file_id,
            position as i16,
        ))
    }
}
