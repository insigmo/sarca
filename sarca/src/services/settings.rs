use sqlx::PgPool;

use crate::{
    errors::SarcaResult,
    repositories::app_settings::AppSettingsRepository,
    schemas::settings::TrashSettingsSchema,
};

pub struct SettingsService<'d> {
    repo: AppSettingsRepository<'d>,
}

impl<'d> SettingsService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            repo: AppSettingsRepository::new(db),
        }
    }

    pub async fn get_trash(&self) -> SarcaResult<TrashSettingsSchema> {
        Ok(TrashSettingsSchema {
            retention_days: self.repo.get_trash_retention_days().await?,
        })
    }

    pub async fn set_trash(&self, retention_days: i32) -> SarcaResult<TrashSettingsSchema> {
        self.repo.set_trash_retention_days(retention_days).await?;
        self.get_trash().await
    }
}
