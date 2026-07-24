use std::time::Duration;

use sqlx::PgPool;

use crate::repositories::{
    app_settings::AppSettingsRepository, files::FilesRepository,
};

use super::trash::purge_file_ids;

pub struct TrashPurgeService;

impl TrashPurgeService {
    pub fn spawn_loop(
        db: PgPool,
        base_url: String,
        rate_limit: u8,
        interval: Duration,
    ) {
        tokio::spawn(async move {
            loop {
                if let Err(e) = Self::run_once(&db, &base_url, rate_limit).await {
                    tracing::warn!("[TRASH PURGE] cycle failed: {e}");
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    pub async fn run_once(db: &PgPool, base_url: &str, rate_limit: u8) -> Result<(), String> {
        let settings = AppSettingsRepository::new(db);
        let retention = settings
            .get_trash_retention_days()
            .await
            .map_err(|e| e.to_string())?;

        let files_repo = FilesRepository::new(db);
        let expired = files_repo
            .list_expired_trashed_ids(retention)
            .await
            .map_err(|e| e.to_string())?;

        if expired.is_empty() {
            return Ok(());
        }

        let ids: Vec<_> = expired.into_iter().map(|(id, _)| id).collect();
        tracing::info!("[TRASH PURGE] permanently deleting {} expired file(s)", ids.len());
        purge_file_ids(db, base_url, rate_limit, &ids)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
