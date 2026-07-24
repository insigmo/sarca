use std::time::Duration;

use sqlx::PgPool;
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
    pub fn new(db: &'d PgPool, rate: u8) -> Self {
        let repo = StorageWorkersRepository::new(db);
        Self {
            repo,
            rate,
        }
    }

    pub async fn get_token(&self, storage_id: Uuid) -> SarcaResult<String> {
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

            // waiting for a while
            tracing::debug!(
                "[TELEGRAM API] waiting for getting a token for a storage with id \"{storage_id}\"",
            );
            sleep(Duration::from_secs(1)).await;
        }
    }
}
