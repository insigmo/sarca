use std::time::Duration;

use sqlx::SqlitePool;

use crate::{
    common::{
        jwt_manager::{AuthUser, JWTManager, TOKEN_TYPE_ACCESS, TOKEN_TYPE_REFRESH},
        password_manager::PasswordManager,
    },
    config::Config,
    errors::{SarcaError, SarcaResult},
    repositories::users::UsersRepository,
    schemas::auth::{LoginSchema, MeSchema, TokenSchema},
};

pub struct AuthService<'d> {
    repo: UsersRepository<'d>,
}

impl<'d> AuthService<'d> {
    pub fn new(db: &'d SqlitePool) -> Self {
        Self {
            repo: UsersRepository::new(db),
        }
    }

    pub async fn login(
        &self,
        login_data: LoginSchema,
        config: &Config,
    ) -> SarcaResult<TokenSchema> {
        let user = self
            .repo
            .get_by_email(&login_data.email)
            .await
            .map_err(|_| SarcaError::NotAuthenticated)?;

        // A disabled account must look like a bad credential, not a distinct
        // error, or the login form leaks which addresses exist and are banned.
        if user.is_disabled() {
            return Err(SarcaError::NotAuthenticated);
        }

        let Some(ref hash) = user.password_hash else {
            return Err(SarcaError::NotAuthenticated);
        };
        PasswordManager::verify(&login_data.password, hash)?;

        let email_verified = user.email_verified();
        let auth = AuthUser::new(user.id, user.email);
        Ok(Self::issue_tokens(auth, email_verified, config))
    }

    pub async fn refresh(&self, refresh_token: &str, config: &Config) -> SarcaResult<TokenSchema> {
        let auth = JWTManager::validate_refresh(refresh_token, &config.secret_key)?;
        // Resolve by `sub`, not by the claim email: an account deleted and later
        // recreated under the same address would otherwise accept the old
        // account's refresh token.
        let user = self.repo.get_by_id(auth.id).await.map_err(|_| SarcaError::NotAuthenticated)?;
        if !user.email.eq_ignore_ascii_case(&auth.email)
            || !user.session_is_live(auth.issued_at)
            || user.is_disabled()
        {
            return Err(SarcaError::NotAuthenticated);
        }
        // Re-issue from the DB row, never from the old claims.
        let email_verified = user.email_verified();
        let auth = AuthUser::new(user.id, user.email);
        Ok(Self::issue_tokens(auth, email_verified, config))
    }

    /// Invalidate every token already issued for this user.
    pub async fn logout(&self, user: &AuthUser) -> SarcaResult<()> {
        self.repo.revoke_sessions(user.id).await
    }

    pub async fn me(&self, user: &AuthUser, config: &Config) -> SarcaResult<MeSchema> {
        let u = self.repo.get_by_id(user.id).await?;
        Ok(MeSchema {
            email_verified: u.email_verified(),
            is_superuser: u.email.eq_ignore_ascii_case(&config.superuser_email),
            email: u.email,
        })
    }

    pub fn issue_tokens(user: AuthUser, email_verified: bool, config: &Config) -> TokenSchema {
        let access_expire = Duration::from_secs(config.access_token_expire_in_secs.into());
        let refresh_expire =
            Duration::from_secs(u64::from(config.refresh_token_expire_in_days) * 24 * 3600);

        let access_token = JWTManager::generate(
            user.clone(),
            access_expire,
            &config.secret_key,
            TOKEN_TYPE_ACCESS,
        );
        let refresh_token =
            JWTManager::generate(user, refresh_expire, &config.secret_key, TOKEN_TYPE_REFRESH);

        TokenSchema::new(access_token, refresh_token, email_verified)
    }
}
