use std::time::Duration;

use sqlx::PgPool;

use crate::{
    common::{
        jwt_manager::{AuthUser, JWTManager, TOKEN_TYPE_ACCESS, TOKEN_TYPE_REFRESH},
        password_manager::PasswordManager,
    },
    config::Config,
    errors::{SarcaError, SarcaResult},
    repositories::users::UsersRepository,
    schemas::auth::{LoginSchema, TokenSchema},
};

pub struct AuthService<'d> {
    repo: UsersRepository<'d>,
}

impl<'d> AuthService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        let repo = UsersRepository::new(db);
        Self { repo }
    }

    pub async fn login(
        &self,
        login_data: LoginSchema,
        config: &Config,
    ) -> SarcaResult<TokenSchema> {
        // trying to find a user with a given email
        let user = self
            .repo
            .get_by_email(&login_data.email)
            .await
            .map_err(|_| SarcaError::NotAuthenticated)?;

        // verifying password
        PasswordManager::verify(&login_data.password, &user.password_hash)?;

        let user = AuthUser::new(user.id, login_data.email);
        Ok(Self::issue_tokens(user, config))
    }

    pub async fn refresh(
        &self,
        refresh_token: &str,
        config: &Config,
    ) -> SarcaResult<TokenSchema> {
        let user = JWTManager::validate_refresh(refresh_token, &config.secret_key)?;
        // Ensure the user still exists
        self.repo
            .get_by_email(&user.email)
            .await
            .map_err(|_| SarcaError::NotAuthenticated)?;
        Ok(Self::issue_tokens(user, config))
    }

    fn issue_tokens(user: AuthUser, config: &Config) -> TokenSchema {
        let access_expire = Duration::from_secs(config.access_token_expire_in_secs.into());
        let refresh_expire =
            Duration::from_secs(u64::from(config.refresh_token_expire_in_days) * 24 * 3600);

        let access_token =
            JWTManager::generate(user.clone(), access_expire, &config.secret_key, TOKEN_TYPE_ACCESS);
        let refresh_token =
            JWTManager::generate(user, refresh_expire, &config.secret_key, TOKEN_TYPE_REFRESH);

        TokenSchema::new(access_token, refresh_token)
    }
}
