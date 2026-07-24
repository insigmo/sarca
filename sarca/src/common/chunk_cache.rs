use std::path::{Path, PathBuf};

use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{
    common::telegram_api::bot_api::TelegramBotApi,
    errors::{SarcaError, SarcaResult},
};

/// On-disk cache of Telegram file payloads under `WORK_DIR/chunk_cache`.
///
/// Lets video Range responses reuse a chunk and lets us prefetch the next
/// Telegram document while the player is still consuming the current one.
#[derive(Clone, Debug)]
pub struct ChunkCache {
    root: PathBuf,
}

impl ChunkCache {
    pub fn new(work_dir: impl AsRef<Path>) -> Self {
        Self {
            root: work_dir.as_ref().join("chunk_cache"),
        }
    }

    fn path_for(&self, telegram_file_id: &str) -> PathBuf {
        // Telegram file_ids are long and may contain odd characters; hash for a stable path.
        let digest = {
            use std::{
                collections::hash_map::DefaultHasher,
                hash::{Hash, Hasher},
            };
            let mut h = DefaultHasher::new();
            telegram_file_id.hash(&mut h);
            h.finish()
        };
        self.root.join(format!("{digest:016x}.bin"))
    }

    /// Download from Telegram into the cache if missing; return the local path.
    pub async fn ensure(
        &self,
        telegram_file_id: &str,
        storage_id: Uuid,
        api: &TelegramBotApi<'_>,
    ) -> SarcaResult<PathBuf> {
        tokio::fs::create_dir_all(&self.root).await.map_err(|e| {
            SarcaError::TelegramAPIError(format!("Can't create chunk cache dir: {e}"))
        })?;

        let dest = self.path_for(telegram_file_id);
        if dest.is_file() {
            return Ok(dest);
        }

        let tmp = self.root.join(format!("{}.tmp", Uuid::new_v4()));
        let result = async {
            let mut stream = api.download_stream(telegram_file_id, storage_id).await?;
            let mut file = tokio::fs::File::create(&tmp).await.map_err(|e| {
                SarcaError::TelegramAPIError(format!("Can't create chunk cache temp: {e}"))
            })?;
            while let Some(item) = stream.next().await {
                let bytes = item?;
                file.write_all(&bytes).await.map_err(|e| {
                    SarcaError::TelegramAPIError(format!("Can't write chunk cache: {e}"))
                })?;
            }
            file.flush().await.map_err(|e| {
                SarcaError::TelegramAPIError(format!("Can't flush chunk cache: {e}"))
            })?;
            drop(file);
            // Another request may have finished first.
            if dest.is_file() {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Ok(dest.clone());
            }
            tokio::fs::rename(&tmp, &dest).await.map_err(|e| {
                SarcaError::TelegramAPIError(format!("Can't finalize chunk cache: {e}"))
            })?;
            Ok(dest.clone())
        }
        .await;

        if result.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        result
    }
}
