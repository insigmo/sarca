use sqlx::SqlitePool;
use uuid::Uuid;

use super::storage_workers_scheduler::StorageWorkersScheduler;
use crate::{
    common::{
        channels::{UploadFileData, UploadProgressEvent, emit_upload_progress},
        media_cache::MediaCache,
        telegram_api::bot_api::{TelegramBotApi, UploadFilePartRequest},
        types::ChatId,
    },
    errors::{SarcaError, SarcaResult},
    models::{
        chunk_replicas::ChunkReplica,
        file_chunks::FileChunk,
        files::File,
        storage_channels::StorageChannel,
    },
    repositories::{
        chunk_replicas::ChunkReplicasRepository,
        files::FilesRepository,
        storage_channels::StorageChannelsRepository,
        storages::StoragesRepository,
    },
    services::{thumbnails, thumbnails::ThumbAndPreview},
};

pub struct StorageManagerService<'d> {
    storages_repo: StoragesRepository<'d>,
    channels_repo: StorageChannelsRepository<'d>,
    files_repo: FilesRepository<'d>,
    replicas_repo: ChunkReplicasRepository<'d>,
    telegram_baseurl: &'d str,
    db: &'d SqlitePool,
    rate_limit: u8,
    /// `WORK_DIR`, so freshly built previews can warm the on-disk preview cache.
    work_dir: &'d str,
}

enum PreviewSource {
    /// Built alongside the thumbnail off the same decode (image) or the same
    /// extracted keyframe (video) — use it directly, no second pass.
    Ready(Vec<u8>),
    /// An image whose thumbnail step produced nothing (failed, or the file was
    /// uploaded before previews were derived there) — generate from disk.
    NeedsGeneration,
    /// Not an image and nothing precomputed (e.g. video keyframe
    /// extraction failed) — no preview for this file.
    Skip,
}

fn resolve_preview_bytes(precomputed: Option<Vec<u8>>, is_image: bool) -> PreviewSource {
    match (precomputed, is_image) {
        (Some(bytes), _) => PreviewSource::Ready(bytes),
        (None, true) => PreviewSource::NeedsGeneration,
        (None, false) => PreviewSource::Skip,
    }
}

impl<'d> StorageManagerService<'d> {
    pub fn new(
        db: &'d SqlitePool,
        telegram_baseurl: &'d str,
        rate_limit: u8,
        work_dir: &'d str,
    ) -> Self {
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
            work_dir,
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
        // Always unlink the WORK_DIR spool when this future completes or is aborted
        // (cancel / early `?` used to leave orphans until process restart).
        struct RemoveSpool(std::path::PathBuf);
        impl Drop for RemoveSpool {
            fn drop(&mut self) {
                if let Err(e) = std::fs::remove_file(&self.0) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!("failed to remove upload spool {}: {e}", self.0.display());
                    }
                }
            }
        }
        let _spool_guard = RemoveSpool(data.file_path.clone());

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
            // Never await progress: a stuck NDJSON client must not freeze SM.
            emit_upload_progress(tx, UploadProgressEvent::telegram(0, total, 1, total_chunks))?;
        }

        while offset < total {
            if data.progress.as_ref().is_some_and(tokio::sync::mpsc::Sender::is_closed) {
                return Err(SarcaError::TelegramAPIError("Upload canceled".to_owned()));
            }
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
                emit_upload_progress(
                    tx,
                    UploadProgressEvent::telegram(
                        offset,
                        total,
                        chunk_no.min(total_chunks),
                        total_chunks,
                    ),
                )?;
            }
        }

        self.files_repo.create_chunks_batch(chunks).await?;
        let result = self.replicas_repo.insert_batch(replicas).await;

        if result.is_ok() {
            // Both derived assets need the same row (logical path, size, chunk size);
            // read it once and hand it down instead of querying per asset.
            match self.files_repo.get_by_id(data.file_id).await {
                Ok(file) => {
                    let derived_preview = match self
                        .maybe_upload_thumb(
                            &file,
                            storage.id,
                            primary.chat_id,
                            &data.file_path,
                            data.client_thumb,
                        )
                        .await
                    {
                        Ok(preview) => preview,
                        Err(e) => {
                            tracing::warn!("thumbnail upload failed for {}: {e}", data.file_id);
                            None
                        },
                    };
                    if let Err(e) = self
                        .maybe_upload_preview(
                            &file,
                            storage.id,
                            primary.chat_id,
                            &data.file_path,
                            derived_preview,
                        )
                        .await
                    {
                        tracing::warn!("preview upload failed for {}: {e}", data.file_id);
                    }
                },
                Err(e) => {
                    tracing::warn!("derived assets skipped for {}: {e}", data.file_id);
                },
            }
            // Mark uploaded here so a client disconnect after Telegram finishes (oneshot
            // already closed) still leaves a visible file instead of a stale spool row.
            if let Err(e) = self.files_repo.set_as_uploaded(data.file_id).await {
                tracing::error!("set_as_uploaded failed for {}: {e}", data.file_id);
                return Err(e);
            }
        }

        result
    }

    /// Store the grid thumbnail. Returns the screen-sized preview when one was
    /// derived on the way (video keyframe, or the fallback decode below).
    ///
    /// Thumbnails belong to the client: it holds the picture already, so it
    /// downscales and sends the tile with the upload. The server only decodes
    /// when nothing arrived — a video or PDF, an API client, an old build.
    async fn maybe_upload_thumb(
        &self,
        file: &File,
        storage_id: Uuid,
        chat_id: ChatId,
        file_path: &std::path::Path,
        client_thumb: Option<Vec<u8>>,
    ) -> SarcaResult<Option<Vec<u8>>> {
        let file_id = file.id;

        let result = match client_thumb {
            Some(thumb) => {
                ThumbAndPreview {
                    thumb,
                    preview: None,
                }
            },
            None => {
                match thumbnails::generate(
                    file_path,
                    &file.path,
                    file.chunk_size_bytes.and_then(|n| u64::try_from(n).ok()).unwrap_or(u64::MAX),
                )
                .await
                {
                    Ok(Some(result)) => result,
                    Ok(None) => return Ok(None),
                    Err(e) => {
                        tracing::warn!("thumbnail generation failed: {e}");
                        return Ok(None);
                    },
                }
            },
        };

        let scheduler = StorageWorkersScheduler::new(self.db, self.rate_limit);
        let outcome = TelegramBotApi::new(self.telegram_baseurl, scheduler)
            .upload(&result.thumb, chat_id, storage_id)
            .await?;

        self.files_repo.set_thumb(file_id, &outcome.file_id, outcome.message_id).await?;

        tracing::debug!(
            "uploaded thumbnail for file {} as telegram_file_id {} (message_id={})",
            file_id,
            outcome.file_id,
            outcome.message_id
        );

        Ok(result.preview)
    }

    /// Build the screen-sized JPEG preview for an image while the original is still on
    /// disk, store it as its own Telegram document, and warm the local preview cache.
    ///
    /// Opening a photo then costs one small `getFile` (or a disk read) instead of
    /// re-downloading every chunk of the original and re-encoding it.
    async fn maybe_upload_preview(
        &self,
        file: &File,
        storage_id: Uuid,
        chat_id: ChatId,
        file_path: &std::path::Path,
        precomputed_preview: Option<Vec<u8>>,
    ) -> SarcaResult<()> {
        let file_id = file.id;

        let jpeg = match resolve_preview_bytes(
            precomputed_preview,
            thumbnails::is_preview_image(&file.path),
        ) {
            PreviewSource::Ready(bytes) => bytes,
            PreviewSource::Skip => return Ok(()),
            PreviewSource::NeedsGeneration => {
                match thumbnails::generate_preview_from_path(file_path).await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::warn!("preview generation failed for {}: {e}", file.path);
                        return Ok(());
                    },
                }
            },
        };

        let cache = MediaCache::previews(self.work_dir);
        let cache_key = cache.key(storage_id, &file.path);
        if let Err(e) = cache.put(&cache_key, &jpeg).await {
            tracing::warn!("preview cache warm skipped for {}: {e}", file.path);
        }

        // An already-small photo gains nothing from a second copy in Telegram: reading
        // its single original chunk costs the same round trip as reading a preview
        // document would. Keep the warm cache entry, skip the upload (and the bot's
        // send budget); the preview endpoint re-encodes on demand if the cache is lost.
        let original_size = file.size.max(0).cast_unsigned();
        if u64::try_from(jpeg.len()).unwrap_or(u64::MAX) * 10 >= original_size * 7 {
            tracing::debug!(
                "preview for {} not stored: {} bytes vs {original_size} byte original",
                file.path,
                jpeg.len()
            );
            return Ok(());
        }

        let scheduler = StorageWorkersScheduler::new(self.db, self.rate_limit);
        let outcome = TelegramBotApi::new(self.telegram_baseurl, scheduler)
            .upload(&jpeg, chat_id, storage_id)
            .await?;

        self.files_repo.set_preview(file_id, &outcome.file_id, outcome.message_id).await?;

        tracing::debug!(
            "uploaded preview for file {} ({} bytes) as telegram_file_id {} (message_id={})",
            file_id,
            jpeg.len(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_preview_bytes_prefers_precomputed_video_bytes() {
        let source = resolve_preview_bytes(Some(vec![1, 2, 3]), true);
        assert!(matches!(source, PreviewSource::Ready(bytes) if bytes == vec![1, 2, 3]));
    }

    #[test]
    fn resolve_preview_bytes_generates_for_images_without_precomputed() {
        let source = resolve_preview_bytes(None, true);
        assert!(matches!(source, PreviewSource::NeedsGeneration));
    }

    #[test]
    fn resolve_preview_bytes_skips_non_images_without_precomputed() {
        let source = resolve_preview_bytes(None, false);
        assert!(matches!(source, PreviewSource::Skip));
    }
}
