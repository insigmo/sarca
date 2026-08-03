use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    common::{
        jwt_manager::{AuthUser, JWTManager, TOKEN_TYPE_ACCESS, TOKEN_TYPE_REFRESH},
        mailer::{self, Mailer},
        password_manager::PasswordManager,
    },
    config::Config,
    errors::{SarcaError, SarcaResult},
    models::email_tokens::{PURPOSE_RESET, PURPOSE_VERIFY},
    repositories::{email_tokens::EmailTokensRepository, users::UsersRepository},
    schemas::auth::{LoginSchema, MeSchema, TokenSchema},
};

const VERIFY_TTL_HOURS: i64 = 48;
const RESET_TTL_HOURS: i64 = 1;

pub struct AuthService<'d> {
    repo: UsersRepository<'d>,
    tokens: EmailTokensRepository<'d>,
}

impl<'d> AuthService<'d> {
    pub fn new(db: &'d SqlitePool) -> Self {
        Self {
            repo: UsersRepository::new(db),
            tokens: EmailTokensRepository::new(db),
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
        if !user.email.eq_ignore_ascii_case(&auth.email) || !user.session_is_live(auth.issued_at) {
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

    pub async fn request_verify(&self, user: &AuthUser, config: &Config) -> SarcaResult<()> {
        Mailer::new(config).require_configured()?;
        let u = self.repo.get_by_id(user.id).await?;
        if u.email_verified() {
            return Ok(());
        }
        self.send_verify_email(u.id, &u.email, config).await
    }

    /// Create verify token and send mail (requires SMTP). Used by resend.
    pub async fn send_verify_email(
        &self,
        user_id: Uuid,
        email: &str,
        config: &Config,
    ) -> SarcaResult<()> {
        Mailer::new(config).require_configured()?;
        let raw = Self::new_raw_token();
        let hash = Self::hash_token(&raw);
        let expires_at = Utc::now() + ChronoDuration::hours(VERIFY_TTL_HOURS);

        self.tokens.invalidate_unused(user_id, PURPOSE_VERIFY).await?;
        self.tokens.create(user_id, PURPOSE_VERIFY, &hash, expires_at).await?;

        let (subject, text, html) = mailer::verify_email_body(&config.public_base_url, &raw);
        Mailer::new(config).send(email, &subject, &text, &html).await
    }

    pub async fn verify_token(&self, raw_token: &str) -> SarcaResult<()> {
        let hash = Self::hash_token(raw_token);
        let token = self
            .tokens
            .get_valid_by_hash(&hash, PURPOSE_VERIFY)
            .await
            .map_err(|_| SarcaError::InvalidToken)?;
        self.repo.mark_email_verified(token.user_id).await?;
        self.tokens.mark_used(token.id).await?;
        Ok(())
    }

    /// Always succeeds from the caller's perspective (no email enumeration).
    pub async fn forgot_password(&self, email: &str, config: &Config) {
        if !config.smtp_configured() {
            return;
        }
        let Ok(user) = self.repo.get_by_email(email).await else {
            return;
        };

        let raw = Self::new_raw_token();
        let hash = Self::hash_token(&raw);
        let expires_at = Utc::now() + ChronoDuration::hours(RESET_TTL_HOURS);

        if let Err(e) = self.tokens.invalidate_unused(user.id, PURPOSE_RESET).await {
            tracing::warn!("reset token invalidate failed: {e}");
            return;
        }
        if let Err(e) = self.tokens.create(user.id, PURPOSE_RESET, &hash, expires_at).await {
            tracing::warn!("reset token create failed: {e}");
            return;
        }

        let (subject, text, html) = mailer::reset_email_body(&config.public_base_url, &raw);
        Mailer::new(config).send_soft(&user.email, &subject, &text, &html).await;
    }

    pub async fn reset_password(&self, raw_token: &str, new_password: &str) -> SarcaResult<()> {
        let hash = Self::hash_token(raw_token);
        let token = self
            .tokens
            .get_valid_by_hash(&hash, PURPOSE_RESET)
            .await
            .map_err(|_| SarcaError::InvalidToken)?;

        let password_hash = PasswordManager::generate(new_password)?;
        self.repo.update_password_hash_by_id(token.user_id, &password_hash).await?;
        // Completing reset also verifies email (they proved inbox access).
        self.repo.mark_email_verified(token.user_id).await?;
        self.tokens.mark_used(token.id).await?;
        Ok(())
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

    pub fn hash_token(raw: &str) -> String {
        use std::fmt::Write;
        let digest = Sha256::digest(raw.as_bytes());
        let mut out = String::with_capacity(digest.len() * 2);
        for b in digest {
            let _ = write!(out, "{b:02x}");
        }
        out
    }

    fn new_raw_token() -> String {
        Uuid::new_v4().to_string()
    }
}
