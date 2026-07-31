use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    common::{jwt_manager::AuthUser, password_manager::PasswordManager},
    config::Config,
    errors::{SarcaError, SarcaResult},
    models::users::InDBUser,
    repositories::{
        access::AccessRepository,
        storages::StoragesRepository,
        users::UsersRepository,
    },
    schemas::users::{InUser, UserOut},
    services::storage_purge::{
        enqueue_storage_telegram_purge_in_tx,
        snapshot_storage_telegram_purge,
    },
};

pub struct UsersService<'d> {
    repo: UsersRepository<'d>,
    db: &'d SqlitePool,
}

impl<'d> UsersService<'d> {
    pub fn new(db: &'d SqlitePool) -> Self {
        Self {
            repo: UsersRepository::new(db),
            db,
        }
    }

    fn require_superuser(user: &AuthUser, config: &Config) -> SarcaResult<()> {
        if user.email.eq_ignore_ascii_case(&config.superuser_email) {
            Ok(())
        } else {
            Err(SarcaError::Forbidden)
        }
    }

    pub async fn list_for_superuser(
        &self,
        actor: &AuthUser,
        config: &Config,
    ) -> SarcaResult<Vec<UserOut>> {
        Self::require_superuser(actor, config)?;
        let users = self.repo.list_all().await?;
        Ok(users
            .into_iter()
            .map(|u| {
                UserOut {
                    is_superuser: u.email.eq_ignore_ascii_case(&config.superuser_email),
                    email_verified: u.email_verified(),
                    id: u.id,
                    email: u.email,
                }
            })
            .collect())
    }

    pub async fn create_by_superuser(
        &self,
        actor: &AuthUser,
        in_user: InUser,
        config: &Config,
    ) -> SarcaResult<()> {
        Self::require_superuser(actor, config)?;
        let password_hash = PasswordManager::generate(&in_user.password).unwrap();
        let mut user = InDBUser::new_password(in_user.email, password_hash);
        // Admin-created accounts are trusted; skip email verification gate.
        user.email_verified_at = Some(Utc::now());
        self.repo.create(user).await?;
        Ok(())
    }

    /// Delete a user, their bots and grants, and every storage they alone owned.
    ///
    /// `access` and `storage_workers` rows cascade with the user row, but storages have
    /// no owner column: a storage whose only admin is the deleted user would survive
    /// invisible to everyone, so it is purged through the normal storage-delete path
    /// (Telegram messages included). Storages shared with someone else are kept.
    pub async fn delete_by_superuser(
        &self,
        actor: &AuthUser,
        user_id: Uuid,
        config: &Config,
    ) -> SarcaResult<()> {
        Self::require_superuser(actor, config)?;

        let target = self.repo.get_by_id(user_id).await?;
        if target.email.eq_ignore_ascii_case(&config.superuser_email) {
            return Err(SarcaError::CannotDeleteSuperuser);
        }
        if target.id == actor.id {
            return Err(SarcaError::CannotDeleteSuperuser);
        }

        let access_repo = AccessRepository::new(self.db);
        let storages = StoragesRepository::new(self.db).list_by_user_id(user_id).await?;
        let mut orphaned = Vec::new();
        for storage in storages {
            let holders = access_repo.list_users_with_access(storage.id).await?;
            if holders.iter().all(|u| u.id == user_id) {
                orphaned.push(storage.id);
            }
        }

        for storage_id in orphaned {
            let snapshot = snapshot_storage_telegram_purge(self.db, storage_id).await?;
            let mut tx = self.db.begin().await.map_err(|e| {
                tracing::error!("[USERS SERVICE] failed to begin storage delete transaction: {e}");
                SarcaError::Unknown
            })?;
            enqueue_storage_telegram_purge_in_tx(&mut tx, storage_id, snapshot).await?;
            StoragesRepository::new(self.db).delete_storage_in_tx(&mut tx, storage_id).await?;
            tx.commit().await.map_err(|e| {
                tracing::error!("[USERS SERVICE] failed to commit storage delete: {e}");
                SarcaError::Unknown
            })?;
            tracing::info!(
                "[USERS SERVICE] deleted storage {storage_id} orphaned by user {user_id}"
            );
        }

        self.repo.delete(user_id).await?;
        tracing::info!("[USERS SERVICE] deleted user {user_id} ({})", target.email);
        Ok(())
    }
}
