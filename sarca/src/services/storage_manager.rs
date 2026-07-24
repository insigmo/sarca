use sqlx::PgPool;
use uuid::Uuid;

use super::storage_workers_scheduler::StorageWorkersScheduler;
use crate::{
    common::{
        channels::{UploadFileData, UploadProgressEvent},
        telegram_api::bot_api::{TelegramBotApi, UploadFilePartRequest},
        types::ChatId,
    },
    errors::{SarcaError, SarcaResult},
    models::{
        chunk_replicas::ChunkReplica,
        file_chunks::FileChunk,
        storage_channels::StorageChannel,
    },
    repositories::{
        chunk_replicas::ChunkReplicasRepository,
        files::FilesRepository,
        storage_channels::StorageChannelsRepository,
        storages::StoragesRepository,
    },
    services::thumbnails,
};

pub struct StorageManagerService<'d> {
    storages_repo: StoragesRepository<'d>,
    channels_repo: StorageChannelsRepository<'d>,
    files_repo: FilesRepository<'d>,
    replicas_repo: ChunkReplicasRepository<'d>,
    telegram_baseurl: &'d str,
    db: &'d PgPool,
    rate_limit: u8,
}

impl<'d> StorageManagerService<'d> {
    pub fn new(db: &'d PgPool, telegram_baseurl: &'d str, rate_limit: u8) -> Self {
        let files_repo = FilesRepository::new(db);
        let storages_repo = StoragesRepository::new(db);
        let channels_repo = StorageChannelsRepository::new(db);
        let replicas_repo = ChunkReplicasRepository::new(db);
        Self {
            storages_repo,
            channels_repo,
            files_repo,
            replicas_repo,
            telegram_baseurl,
            db,
            rate_limit,
        }
    }

    /// Pick the primary channel to upload to: the storage's `primary_position` if still
    /// active, otherwise the first active channel (and persist the rotation).
    async fn resolve_primary_channel(
        &self,
        storage_id: Uuid,
        primary_position: i16,
    ) -> SarcaResult<(StorageChannel, Vec<StorageChannel>)> {
        let channels = self.channels_repo.list_by_storage(storage_id).await?;
        let active: Vec<StorageChannel> =
            channels.iter().filter(|c| c.is_active()).cloned().collect();

        let Some(primary) = active
            .iter()
            .find(|c| c.position == primary_position)
            .cloned()
            .or_else(|| active.first().cloned())
        else {
            return Err(SarcaError::NoActiveChannel);
        };

        if primary.position != primary_position {
            let _ = self.storages_repo.set_primary_position(storage_id, primary.position).await;
        }

        Ok((primary, active))
    }

    pub async fn upload(&self, data: UploadFileData) -> SarcaResult<()> {
        let storage = self.storages_repo.get_by_file_id(data.file_id).await?;
        let (primary, active_channels) =
            self.resolve_primary_channel(storage.id, storage.primary_position).await?;
        let secondary_channels: Vec<StorageChannel> =
            active_channels.into_iter().filter(|c| c.id != primary.id).collect();

        let mut position: usize = 0;
        let mut chunks: Vec<FileChunk> = Vec::new();
        let mut replicas: Vec<ChunkReplica> = Vec::new();

        let mut offset: u64 = 0;
        let total: u64 = data.file_size.max(0).cast_unsigned();
        let chunk_size = data.chunk_size.max(1) as u64;
        let total_chunks = if total == 0 {
            1u32
        } else {
            u32::try_from(total.div_ceil(chunk_size)).unwrap_or(u32::MAX)
        };

        if let Some(tx) = data.progress.as_ref() {
            let _ = tx.send(UploadProgressEvent::telegram(0, total, 1, total_chunks)).await;
        }

        while offset < total {
            let len = std::cmp::min(chunk_size, total - offset);
            let chunk_no = u32::try_from(position).unwrap_or(u32::MAX).saturating_add(1);
            let (chunk, replica) = self
                .upload_chunk_from_file(
                    storage.id,
                    primary.id,
                    primary.chat_id,
                    data.file_id,
                    position,
                    &data.file_path,
                    offset,
                    len,
                    total,
                    chunk_no,
                    total_chunks,
                    data.progress.clone(),
                )
                .await?;

            for secondary in &secondary_channels {
                replicas.push(ChunkReplica::new_pending(Uuid::new_v4(), chunk.id, secondary.id));
            }

            chunks.push(chunk);
            replicas.push(replica);
            offset += len;
            position += 1;
            if let Some(tx) = data.progress.as_ref() {
                let _ = tx
                    .send(UploadProgressEvent::telegram(
                        offset,
                        total,
                        chunk_no.min(total_chunks),
                        total_chunks,
                    ))
                    .await;
            }
        }

        self.files_repo.create_chunks_batch(chunks).await?;
        let result = self.replicas_repo.insert_batch(replicas).await;

        if result.is_ok() {
            if let Err(e) = self
                .maybe_upload_thumb(data.file_id, storage.id, primary.chat_id, &data.file_path)
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
            },
        };

        let scheduler = StorageWorkersScheduler::new(self.db, self.rate_limit);
        let outcome = TelegramBotApi::new(self.telegram_baseurl, scheduler)
            .upload(&jpeg, chat_id, storage_id)
            .await?;

        self.files_repo.set_thumb(file_id, &outcome.file_id, outcome.message_id).await?;

        tracing::debug!(
            "uploaded thumbnail for file {} as telegram_file_id {} (message_id={})",
            file_id,
            outcome.file_id,
            outcome.message_id
        );

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn upload_chunk_from_file(
        &self,
        storage_id: Uuid,
        channel_id: Uuid,
        chat_id: ChatId,
        file_id: Uuid,
        position: usize,
        file_path: &std::path::Path,
        offset: u64,
        len: u64,
        file_total: u64,
        chunk_no: u32,
        total_chunks: u32,
        progress: Option<tokio::sync::mpsc::Sender<UploadProgressEvent>>,
    ) -> SarcaResult<(FileChunk, ChunkReplica)> {
        let scheduler = StorageWorkersScheduler::new(self.db, self.rate_limit);

        let outcome = TelegramBotApi::new(self.telegram_baseurl, scheduler)
            .upload_file_part(
                file_path,
                UploadFilePartRequest {
                    offset,
                    len,
                    chat_id,
                    storage_id,
                    file_total,
                    chunk_no,
                    total_chunks,
                    progress,
                },
            )
            .await?;

        tracing::debug!(
            "[TELEGRAM API] uploaded chunk with file_id \"{}\" and position \"{}\"",
            outcome.file_id,
            position
        );

        let chunk_id = Uuid::new_v4();
        let chunk = FileChunk::new(chunk_id, file_id, i16::try_from(position).unwrap_or(i16::MAX));
        let replica = ChunkReplica::new_uploaded(
            Uuid::new_v4(),
            chunk_id,
            channel_id,
            outcome.file_id,
            outcome.message_id,
        );

        Ok((chunk, replica))
    }
}
