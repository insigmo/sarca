use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::db::errors::map_not_found,
    errors::{SarcaError, SarcaResult},
    models::users::{InDBUser, User},
};

pub struct UsersRepository<'d> {
    db: &'d PgPool,
}

impl<'d> UsersRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            db,
        }
    }

    pub async fn create(&self, in_obj: InDBUser) -> SarcaResult<User> {
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
                INSERT INTO users (id, email, password_hash, email_verified_at)
                VALUES ($1, $2, $3, $4);
            "#,
        )
        .bind(id)
        .bind(in_obj.email.clone())
        .bind(in_obj.password_hash.clone())
        .bind(in_obj.email_verified_at)
        .execute(self.db)
        .await
        .map_err(|e| {
            match e {
                sqlx::Error::Database(dbe) if dbe.constraint() == Some("users_email_key") => {
                    SarcaError::AlreadyExists("user with given email".into())
                },
                _ => {
                    tracing::error!("{e}");
                    SarcaError::Unknown
                },
            }
        })?;

        Ok(User {
            id,
            email: in_obj.email,
            password_hash: in_obj.password_hash,
            email_verified_at: in_obj.email_verified_at,
        })
    }

    pub async fn get_by_email(&self, email: &str) -> SarcaResult<User> {
        sqlx::query_as("SELECT * FROM users WHERE email = $1")
            .bind(email)
            .fetch_one(self.db)
            .await
            .map_err(|e| map_not_found(&e, "user"))
    }

    pub async fn get_by_id(&self, id: Uuid) -> SarcaResult<User> {
        sqlx::query_as("SELECT * FROM users WHERE id = $1")
            .bind(id)
            .fetch_one(self.db)
            .await
            .map_err(|e| map_not_found(&e, "user"))
    }

    pub async fn update_password_hash(&self, email: &str, password_hash: &str) -> SarcaResult<()> {
        let res = sqlx::query("UPDATE users SET password_hash = $2 WHERE email = $1")
            .bind(email)
            .bind(password_hash)
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;
        if res.rows_affected() == 0 {
            return Err(SarcaError::DoesNotExist("user".into()));
        }
        Ok(())
    }

    pub async fn update_password_hash_by_id(
        &self,
        user_id: Uuid,
        password_hash: &str,
    ) -> SarcaResult<()> {
        let res = sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(user_id)
            .bind(password_hash)
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;
        if res.rows_affected() == 0 {
            return Err(SarcaError::DoesNotExist("user".into()));
        }
        Ok(())
    }

    pub async fn mark_email_verified(&self, user_id: Uuid) -> SarcaResult<()> {
        let now: DateTime<Utc> = Utc::now();
        let res = sqlx::query(
            "UPDATE users SET email_verified_at = COALESCE(email_verified_at, $2) WHERE id = $1",
        )
        .bind(user_id)
        .bind(now)
        .execute(self.db)
        .await
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        if res.rows_affected() == 0 {
            return Err(SarcaError::DoesNotExist("user".into()));
        }
        Ok(())
    }
}
