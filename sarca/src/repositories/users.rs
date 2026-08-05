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
            sessions_valid_after: 0,
            disabled_at: None,
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
        // Changing the password must also evict every token already issued for
        // this account, otherwise a stolen refresh token survives the reset.
        let res = sqlx::query(
            "UPDATE users SET password_hash = $2, sessions_valid_after = $3 WHERE id = $1",
        )
        .bind(user_id)
        .bind(password_hash)
        .bind(Utc::now().timestamp())
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

    /// Disable or re-enable an account. Disabling also bumps
    /// `sessions_valid_after`, so any session already issued dies immediately
    /// rather than surviving until the token's natural expiry.
    pub async fn set_disabled(&self, user_id: Uuid, disabled: bool) -> SarcaResult<()> {
        let res = if disabled {
            sqlx::query(
                "UPDATE users SET disabled_at = $2, sessions_valid_after = $3 WHERE id = $1",
            )
            .bind(user_id)
            .bind(Utc::now())
            .bind(Utc::now().timestamp())
            .execute(self.db)
            .await
        } else {
            sqlx::query("UPDATE users SET disabled_at = NULL WHERE id = $1")
                .bind(user_id)
                .execute(self.db)
                .await
        }
        .map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })?;
        if res.rows_affected() == 0 {
            return Err(SarcaError::DoesNotExist("user".into()));
        }
        Ok(())
    }

    /// Enabled users only, for the grant-access autocomplete directory.
    pub async fn list_directory(&self) -> SarcaResult<Vec<(Uuid, String)>> {
        sqlx::query_as("SELECT id, email FROM users WHERE disabled_at IS NULL ORDER BY email ASC")
            .fetch_all(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })
    }

    /// Invalidate every token issued so far for this user (logout everywhere).
    pub async fn revoke_sessions(&self, user_id: Uuid) -> SarcaResult<()> {
        sqlx::query("UPDATE users SET sessions_valid_after = $2 WHERE id = $1")
            .bind(user_id)
            .bind(Utc::now().timestamp())
            .execute(self.db)
            .await
            .map_err(|e| {
                tracing::error!("{e}");
                SarcaError::Unknown
            })?;
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
        common::{db::pool::get_pool, jwt_manager::AuthUser, password_manager::PasswordManager},
        config::Config,
        schemas::{auth::LoginSchema, users::SetDisabled},
        services::{auth::AuthService, users::UsersService},
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
            acme_root_ca: None,
            certs_dir: String::new(),
            workers: 1,
            channel_capacity: 8,
            superuser_email: superuser_email.into(),
            superuser_pass: superuser_pass.into(),
            access_token_expire_in_secs: 1800,
            refresh_token_expire_in_days: 14,
            secret_key: "test-secret".into(),
            telegram_api_base_url: "https://api.telegram.org".into(),
            telegram_rate_limit: 60,
            upload_concurrency: 4,
            media_concurrency: 16,
            work_dir: String::new(),
            telegram_chunk_size_mb: 20,
            telegram_video_chunk_size_mb: 20,
            debug_log: false,
            prefetch_enabled: false,
            prefetch_depth: 3,
            prefetch_concurrency: 3,
            prefetch_max_items: 2000,
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

    #[tokio::test]
    async fn disabled_account_cannot_log_in_or_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
        init_db(&pool).await;
        let config = test_config("admin@example.com", "super-secret");

        let email = "someone@example.com";
        let password_hash = PasswordManager::generate("s3cret").unwrap();
        let user = UsersRepository::new(&pool)
            .create(InDBUser::new_password(email.into(), password_hash))
            .await
            .unwrap();

        let tokens = AuthService::new(&pool)
            .login(
                LoginSchema {
                    email: email.into(),
                    password: "s3cret".into(),
                },
                &config,
            )
            .await
            .unwrap();

        UsersRepository::new(&pool).set_disabled(user.id, true).await.unwrap();

        let login_err = AuthService::new(&pool)
            .login(
                LoginSchema {
                    email: email.into(),
                    password: "s3cret".into(),
                },
                &config,
            )
            .await
            .unwrap_err();
        assert!(matches!(login_err, SarcaError::NotAuthenticated), "got {login_err:?}");

        let refresh_err =
            AuthService::new(&pool).refresh(&tokens.refresh_token, &config).await.unwrap_err();
        assert!(matches!(refresh_err, SarcaError::NotAuthenticated), "got {refresh_err:?}");
    }

    #[tokio::test]
    async fn set_disabled_toggles_disabled_at_and_bumps_sessions_valid_after() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
        init_db(&pool).await;

        let user = UsersRepository::new(&pool)
            .create(InDBUser::new_password("someone@example.com".into(), "hash".into()))
            .await
            .unwrap();
        assert_eq!(user.sessions_valid_after, 0);

        UsersRepository::new(&pool).set_disabled(user.id, true).await.unwrap();
        let disabled = UsersRepository::new(&pool).get_by_id(user.id).await.unwrap();
        assert!(disabled.is_disabled());
        assert!(disabled.sessions_valid_after > 0);

        UsersRepository::new(&pool).set_disabled(user.id, false).await.unwrap();
        let enabled = UsersRepository::new(&pool).get_by_id(user.id).await.unwrap();
        assert!(!enabled.is_disabled());
    }

    #[tokio::test]
    async fn superuser_cannot_disable_itself_or_the_configured_superuser_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
        init_db(&pool).await;
        let config = test_config("admin@example.com", "super-secret");
        create_superuser(&pool, &config).await;

        let superuser =
            UsersRepository::new(&pool).get_by_email(&config.superuser_email).await.unwrap();
        let actor = AuthUser::new(superuser.id, superuser.email.clone());

        let err = UsersService::new(&pool)
            .set_disabled_by_superuser(
                &actor,
                superuser.id,
                SetDisabled {
                    disabled: true,
                },
                &config,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, SarcaError::Forbidden), "got {err:?}");
    }

    #[tokio::test]
    async fn list_directory_is_forbidden_for_a_user_with_no_admin_grant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
        init_db(&pool).await;
        let config = test_config("admin@example.com", "super-secret");

        let user = UsersRepository::new(&pool)
            .create(InDBUser::new_password("someone@example.com".into(), "hash".into()))
            .await
            .unwrap();
        let actor = AuthUser::new(user.id, user.email.clone());

        let err = UsersService::new(&pool).list_directory(&actor, &config).await.unwrap_err();
        assert!(matches!(err, SarcaError::Forbidden), "got {err:?}");
    }

    #[tokio::test]
    async fn list_directory_excludes_disabled_users() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
        init_db(&pool).await;
        let config = test_config("admin@example.com", "super-secret");
        create_superuser(&pool, &config).await;

        let active = UsersRepository::new(&pool)
            .create(InDBUser::new_password("active@example.com".into(), "hash".into()))
            .await
            .unwrap();
        let disabled = UsersRepository::new(&pool)
            .create(InDBUser::new_password("disabled@example.com".into(), "hash".into()))
            .await
            .unwrap();
        UsersRepository::new(&pool).set_disabled(disabled.id, true).await.unwrap();

        let superuser =
            UsersRepository::new(&pool).get_by_email(&config.superuser_email).await.unwrap();
        let actor = AuthUser::new(superuser.id, superuser.email.clone());

        let entries = UsersService::new(&pool).list_directory(&actor, &config).await.unwrap();
        assert!(entries.iter().any(|e| e.id == active.id));
        assert!(!entries.iter().any(|e| e.id == disabled.id));
    }
}
