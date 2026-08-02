use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::Semaphore;

use crate::{
    common::{
        channels::{
            ClientData,
            ClientMessage,
            StorageManagerData,
            StorageManagerListener,
            StorageManagerMessage,
            UploadFileData,
        },
        telegram_api::bot_api::flood_active,
    },
    config::Config,
    services::storage_manager::StorageManagerService,
};

pub struct StorageManager {
    rx: StorageManagerListener,
    db: SqlitePool,
    config: Config,
}

impl StorageManager {
    pub fn new(rx: StorageManagerListener, db: SqlitePool, config: Config) -> Self {
        Self {
            rx,
            db,
            config,
        }
    }

    pub async fn run(&mut self) {
        // Files run in parallel, chunks of one file do not: a single `upload()` call
        // still sends its chunks (and thumb) strictly in order. Parallelism is safe
        // because every mutating Telegram call goes through the per-token send gate,
        // which serializes and paces each bot on its own — so this only overlaps work
        // belonging to *different* worker tokens, which is exactly where the time goes.
        let limit = usize::from(self.config.upload_concurrency.max(1));
        let gate = Arc::new(Semaphore::new(limit));

        while let Some(msg) = self.rx.recv().await {
            tracing::debug!("got msg");

            // Adaptive backoff: while any token is inside a flood window, take the
            // whole gate so the in-flight file finishes alone. Telegram's `retry_after`
            // is still honored inside the API layer; this just stops us from spending
            // the wait queuing more concurrent floods.
            let weight = if flood_active() {
                tracing::debug!("flood window active, uploading one file at a time");
                u32::try_from(limit).unwrap_or(u32::MAX)
            } else {
                1
            };

            let Ok(permit) = gate.clone().acquire_many_owned(weight).await else {
                tracing::error!("upload gate closed");
                return;
            };

            let db = self.db.clone();
            let config = self.config.clone();
            tokio::spawn(async move {
                let _permit = permit;
                Self::handle_msg(&db, &config, msg).await;
            });
        }
    }

    async fn handle_msg(db: &SqlitePool, config: &Config, msg: ClientMessage) {
        let result = match msg.data {
            ClientData::UploadFile(data) => Self::upload(db, config, data).await,
        };
        let msg_back = StorageManagerMessage::new(result);
        let _ = msg.tx.send(msg_back);
    }

    async fn upload(db: &SqlitePool, config: &Config, data: UploadFileData) -> StorageManagerData {
        let result = StorageManagerService::new(
            db,
            &config.telegram_api_base_url,
            config.telegram_rate_limit,
            &config.work_dir,
        )
        .upload(data)
        .await;

        StorageManagerData::UploadFile(result)
    }
}
