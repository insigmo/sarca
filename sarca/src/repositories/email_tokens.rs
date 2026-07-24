use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::db::errors::map_not_found,
    errors::{SarcaError, SarcaResult},
    models::email_tokens::EmailToken,
};

pub struct EmailTokensRepository<'d> {
    db: &'d PgPool,
}

impl<'d> EmailTokensRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            db,
        }
    }

    pub async fn invalidate_unused(&self, user_id: Uuid, purpose: &str) -> SarcaResult<()> {
        let now = Utc::now();
        sqlx::query(
            r#"
                UPDATE email_tokens
                SET used_at = $3
                WHERE user_id = $1 AND purpose = $2 AND used_at IS NULL
            "#,
        )
        .bind(user_id)
        .bind(purpose)
        .bind(now)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        Ok(())
    }

    pub async fn create(
        &self,
        user_id: Uuid,
        purpose: &str,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> SarcaResult<EmailToken> {
        let id = Uuid::new_v4();
        let created_at = Utc::now();
        sqlx::query(
            r#"
                INSERT INTO email_tokens (id, user_id, purpose, token_hash, expires_at, created_at)
                VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(purpose)
        .bind(token_hash)
        .bind(expires_at)
        .bind(created_at)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;

        Ok(EmailToken {
            id,
            user_id,
            purpose: purpose.to_owned(),
            token_hash: token_hash.to_owned(),
            expires_at,
            used_at: None,
            created_at,
        })
    }

    pub async fn get_valid_by_hash(
        &self,
        token_hash: &str,
        purpose: &str,
    ) -> SarcaResult<EmailToken> {
        let now = Utc::now();
        sqlx::query_as(
            r#"
                SELECT * FROM email_tokens
                WHERE token_hash = $1
                  AND purpose = $2
                  AND used_at IS NULL
                  AND expires_at > $3
            "#,
        )
        .bind(token_hash)
        .bind(purpose)
        .bind(now)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(&e, "token"))
    }

    pub async fn mark_used(&self, id: Uuid) -> SarcaResult<()> {
        let now = Utc::now();
        sqlx::query("UPDATE email_tokens SET used_at = $2 WHERE id = $1")
            .bind(id)
            .bind(now)
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;
        Ok(())
    }
}
