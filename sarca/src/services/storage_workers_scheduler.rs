use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::time::sleep;
use uuid::Uuid;

use crate::{
    errors::{SarcaError, SarcaResult},
    repositories::storage_workers::StorageWorkersRepository,
};

/// Manages storage workers by limiting their usage
pub struct StorageWorkersScheduler<'d> {
    repo: StorageWorkersRepository<'d>,
    rate: u8,
}

impl<'d> StorageWorkersScheduler<'d> {
    pub fn new(db: &'d SqlitePool, rate: u8) -> Self {
        let repo = StorageWorkersRepository::new(db);
        Self {
            repo,
            rate,
        }
    }

    /// Waits indefinitely for a token. Uploads must never give up early: losing
    /// an upload because the bucket was briefly full is worse than a slow one.
    pub async fn get_token(&self, storage_id: Uuid) -> SarcaResult<String> {
        self.get_token_impl(storage_id, None).await
    }

    /// Like `get_token`, but stops waiting and returns `StorageBusy` once
    /// `deadline` passes. Only interactive read paths (thumb/preview) opt into
    /// this — everything else keeps waiting via `get_token`.
    pub async fn get_token_before(
        &self,
        storage_id: Uuid,
        deadline: Instant,
    ) -> SarcaResult<String> {
        self.get_token_impl(storage_id, Some(deadline)).await
    }

    async fn get_token_impl(
        &self,
        storage_id: Uuid,
        deadline: Option<Instant>,
    ) -> SarcaResult<String> {
        // Distinguish "no workers bound yet" from "all workers rate-limited".
        // Without this check, callers that hit Telegram before attaching a worker
        // (e.g. storage create → getChat for channel title) loop forever and block boot.
        if !self.repo.storage_has_any(storage_id).await? {
            return Err(SarcaError::NoStorageWorkers);
        }

        loop {
            // attempting
            if let Some(schema) = self.repo.get_token(storage_id, self.rate).await? {
                return Ok(schema.token);
            }

            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(SarcaError::StorageBusy);
            }

            // waiting for a while
            tracing::debug!(
                "[TELEGRAM API] waiting for getting a token for a storage with id \"{storage_id}\"",
            );
            sleep(Duration::from_secs(1)).await;
        }
    }
}
