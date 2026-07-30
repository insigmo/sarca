use chrono::Utc;
use sqlx::SqlitePool;

use crate::{
    common::{jwt_manager::AuthUser, password_manager::PasswordManager},
    config::Config,
    errors::{SarcaError, SarcaResult},
    models::users::InDBUser,
    repositories::users::UsersRepository,
    schemas::users::{InUser, UserOut},
};

pub struct UsersService<'d> {
    repo: UsersRepository<'d>,
}

impl<'d> UsersService<'d> {
    pub fn new(db: &'d SqlitePool) -> Self {
        Self {
            repo: UsersRepository::new(db),
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
}
