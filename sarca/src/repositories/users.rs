use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    common::db::errors::map_not_found,
    errors::{SarcaError, SarcaResult},
    models::users::{InDBUser, User},
};

pub struct UsersRepository<'d> {
    db: &'d SqlitePool,
}

impl<'d> UsersRepository<'d> {
    pub fn new(db: &'d SqlitePool) -> Self {
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
                sqlx::Error::Database(dbe) if dbe.is_unique_violation() => {
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

    /// Delete a user row. `access` and `storage_workers` rows cascade.
    pub async fn delete(&self, id: Uuid) -> SarcaResult<()> {
        let res = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
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

    pub async fn list_all(&self) -> SarcaResult<Vec<User>> {
        sqlx::query_as("SELECT * FROM users ORDER BY email ASC").fetch_all(self.db).await.map_err(
            |e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            },
        )
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{
        common::db::pool::get_pool,
        config::Config,
        startup::{create_superuser, init_db},
    };

    fn test_config(superuser_email: &str, superuser_pass: &str) -> Config {
        Config {
            sqlite_path: String::new(),
            port: 8001,
            https_addr: "127.0.0.1:8443".parse().expect("valid addr"),
            acme_http_addr: "127.0.0.1:8080".parse().expect("valid addr"),
            tls_hostname: None,
            acme_directory: String::new(),
            certs_dir: String::new(),
            workers: 1,
            channel_capacity: 8,
            superuser_email: superuser_email.into(),
            superuser_pass: superuser_pass.into(),
            access_token_expire_in_secs: 1800,
            refresh_token_expire_in_days: 14,
            secret_key: "test-secret".into(),
            telegram_api_base_url: "https://api.telegram.org".into(),
            telegram_rate_limit: 18,
            work_dir: String::new(),
            telegram_chunk_size_mb: 20,
            telegram_video_chunk_size_mb: 20,
            public_base_url: "http://127.0.0.1:8001".into(),
            smtp_host: None,
            smtp_port: 587,
            smtp_username: None,
            smtp_password: None,
            smtp_from: String::new(),
            smtp_tls: "starttls".into(),
            debug_log: false,
        }
    }

    #[tokio::test]
    async fn create_duplicate_email_maps_to_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
        init_db(&pool).await;

        let email = "admin@example.com";
        let u1 = InDBUser::new_password(email.into(), "h1".into());
        UsersRepository::new(&pool).create(u1).await.unwrap();

        let u2 = InDBUser::new_password(email.into(), "h2".into());
        let err = UsersRepository::new(&pool).create(u2).await.unwrap_err();
        assert!(matches!(err, SarcaError::AlreadyExists(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn create_superuser_twice_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
        init_db(&pool).await;

        let config = test_config("admin@example.com", "super-secret");
        create_superuser(&pool, &config).await;
        create_superuser(&pool, &config).await;
    }
}
