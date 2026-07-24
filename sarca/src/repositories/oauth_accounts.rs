use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::db::errors::map_not_found;
use crate::errors::{SarcaError, SarcaResult};
use crate::models::oauth_accounts::OAuthAccount;

pub struct OAuthAccountsRepository<'d> {
    db: &'d PgPool,
}

impl<'d> OAuthAccountsRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self { db }
    }

    pub async fn get_by_provider(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> SarcaResult<OAuthAccount> {
        sqlx::query_as(
            "SELECT * FROM oauth_accounts WHERE provider = $1 AND provider_user_id = $2",
        )
        .bind(provider)
        .bind(provider_user_id)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(e, "oauth account"))
    }

    pub async fn create(
        &self,
        user_id: Uuid,
        provider: &str,
        provider_user_id: &str,
    ) -> SarcaResult<OAuthAccount> {
        let id = Uuid::new_v4();
        let created_at = Utc::now();
        sqlx::query(
            r#"
                INSERT INTO oauth_accounts (id, user_id, provider, provider_user_id, created_at)
                VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(provider)
        .bind(provider_user_id)
        .bind(created_at)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(OAuthAccount {
            id,
            user_id,
            provider: provider.to_owned(),
            provider_user_id: provider_user_id.to_owned(),
            created_at,
        })
    }
}
