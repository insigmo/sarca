use sqlx::PgPool;

use crate::errors::{SarcaError, SarcaResult};

pub const TABLE: &str = "app_settings";
pub const TRASH_RETENTION_DAYS_KEY: &str = "trash_retention_days";
pub const DEFAULT_TRASH_RETENTION_DAYS: i32 = 30;
pub const MIN_TRASH_RETENTION_DAYS: i32 = 1;
pub const MAX_TRASH_RETENTION_DAYS: i32 = 30;

pub const TELEGRAM_API_ID_KEY: &str = "telegram_api_id";
pub const TELEGRAM_API_HASH_KEY: &str = "telegram_api_hash";
pub const LOCAL_API_SKIPPED_KEY: &str = "local_api_skipped";

pub struct AppSettingsRepository<'d> {
    db: &'d PgPool,
}

impl<'d> AppSettingsRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            db,
        }
    }

    pub async fn get_value(&self, key: &str) -> SarcaResult<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as(format!("SELECT value FROM {TABLE} WHERE key = $1").as_str())
                .bind(key)
                .fetch_optional(self.db)
                .await
                .map_err(|e| {
                    tracing::error!("{e}");
                    SarcaError::Unknown
                })?;
        Ok(row.map(|(v,)| v))
    }

    pub async fn set_value(&self, key: &str, value: &str) -> SarcaResult<()> {
        sqlx::query(
            format!(
                "
                INSERT INTO {TABLE} (key, value) VALUES ($1, $2)
                ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value
                "
            )
            .as_str(),
        )
        .bind(key)
        .bind(value)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(())
    }

    pub async fn get_trash_retention_days(&self) -> SarcaResult<i32> {
        let raw = self
            .get_value(TRASH_RETENTION_DAYS_KEY)
            .await?
            .unwrap_or_else(|| DEFAULT_TRASH_RETENTION_DAYS.to_string());
        raw.parse::<i32>()
            .map_err(|_| SarcaError::Unknown)
            .map(|days| days.clamp(MIN_TRASH_RETENTION_DAYS, MAX_TRASH_RETENTION_DAYS))
    }

    pub async fn set_trash_retention_days(&self, days: i32) -> SarcaResult<()> {
        if !(MIN_TRASH_RETENTION_DAYS..=MAX_TRASH_RETENTION_DAYS).contains(&days) {
            return Err(SarcaError::InvalidTrashRetention);
        }
        self.set_value(TRASH_RETENTION_DAYS_KEY, &days.to_string()).await
    }

    pub async fn set_telegram_api_credentials(
        &self,
        api_id: &str,
        api_hash: &str,
    ) -> SarcaResult<()> {
        self.set_value(TELEGRAM_API_ID_KEY, api_id).await?;
        self.set_value(TELEGRAM_API_HASH_KEY, api_hash).await
    }

    pub async fn is_local_api_skipped(&self) -> SarcaResult<bool> {
        Ok(self
            .get_value(LOCAL_API_SKIPPED_KEY)
            .await?
            .as_deref()
            .is_some_and(|v| v == "true" || v == "1"))
    }

    pub async fn set_local_api_skipped(&self, skipped: bool) -> SarcaResult<()> {
        self.set_value(LOCAL_API_SKIPPED_KEY, if skipped { "true" } else { "false" }).await
    }
}
