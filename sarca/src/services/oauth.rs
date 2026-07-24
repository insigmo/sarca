//! Google + GitHub OAuth start / callback / one-time exchange.

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::routing::app_state::AppState,
    config::Config,
    errors::{SarcaError, SarcaResult},
    models::{
        oauth_accounts::{PROVIDER_GITHUB, PROVIDER_GOOGLE},
        users::InDBUser,
    },
    repositories::{oauth_accounts::OAuthAccountsRepository, users::UsersRepository},
    schemas::auth::TokenSchema,
    services::auth::AuthService,
};

#[derive(Debug, Clone)]
pub struct OAuthProfile {
    pub provider: String,
    pub provider_user_id: String,
    pub email: String,
    pub email_verified: bool,
}

pub struct OAuthService<'d> {
    users: UsersRepository<'d>,
    oauth: OAuthAccountsRepository<'d>,
}

impl<'d> OAuthService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            users: UsersRepository::new(db),
            oauth: OAuthAccountsRepository::new(db),
        }
    }

    pub fn authorize_url(provider: &str, config: &Config, state: &str) -> SarcaResult<String> {
        let base = config.public_base_url.trim_end_matches('/');
        match provider {
            PROVIDER_GOOGLE => {
                if !config.google_oauth_configured() {
                    return Err(SarcaError::OAuthNotConfigured);
                }
                let client_id = config.oauth_google_client_id.as_deref().unwrap();
                let redirect_raw = format!("{base}/api/auth/oauth/google/callback");
                let redirect = utf8_percent_encode(&redirect_raw, NON_ALPHANUMERIC).to_string();
                let scope = utf8_percent_encode("openid email profile", NON_ALPHANUMERIC).to_string();
                Ok(format!(
                    "https://accounts.google.com/o/oauth2/v2/auth?client_id={client_id}\
                     &redirect_uri={redirect}&response_type=code&scope={scope}&state={state}"
                ))
            }
            PROVIDER_GITHUB => {
                if !config.github_oauth_configured() {
                    return Err(SarcaError::OAuthNotConfigured);
                }
                let client_id = config.oauth_github_client_id.as_deref().unwrap();
                let redirect_raw = format!("{base}/api/auth/oauth/github/callback");
                let redirect = utf8_percent_encode(&redirect_raw, NON_ALPHANUMERIC).to_string();
                let scope = utf8_percent_encode("read:user user:email", NON_ALPHANUMERIC).to_string();
                Ok(format!(
                    "https://github.com/login/oauth/authorize?client_id={client_id}\
                     &redirect_uri={redirect}&scope={scope}&state={state}"
                ))
            }
            _ => Err(SarcaError::OAuthNotConfigured),
        }
    }

    pub async fn exchange_code_for_profile(
        provider: &str,
        code: &str,
        config: &Config,
    ) -> SarcaResult<OAuthProfile> {
        match provider {
            PROVIDER_GOOGLE => Self::google_profile(code, config).await,
            PROVIDER_GITHUB => Self::github_profile(code, config).await,
            _ => Err(SarcaError::OAuthNotConfigured),
        }
    }

    pub async fn link_or_create(&self, profile: &OAuthProfile) -> SarcaResult<(Uuid, String, bool)> {
        // Existing oauth link
        if let Ok(acct) = self
            .oauth
            .get_by_provider(&profile.provider, &profile.provider_user_id)
            .await
        {
            let user = self.users.get_by_id(acct.user_id).await?;
            if profile.email_verified && !user.email_verified() {
                let _ = self.users.mark_email_verified(user.id).await;
            }
            let verified = profile.email_verified || user.email_verified();
            return Ok((user.id, user.email, verified));
        }

        // Email match → link
        if let Ok(user) = self.users.get_by_email(&profile.email).await {
            self.oauth
                .create(user.id, &profile.provider, &profile.provider_user_id)
                .await?;
            if profile.email_verified && !user.email_verified() {
                let _ = self.users.mark_email_verified(user.id).await;
            }
            let verified = profile.email_verified || user.email_verified();
            return Ok((user.id, user.email, verified));
        }

        // Create new OAuth-only user
        let in_user = InDBUser::new_oauth(profile.email.clone(), profile.email_verified);
        let user = self.users.create(in_user).await?;
        self.oauth
            .create(user.id, &profile.provider, &profile.provider_user_id)
            .await?;
        let verified = user.email_verified();
        Ok((user.id, user.email, verified))
    }

    pub async fn complete_login(
        state: &AppState,
        provider: &str,
        code: &str,
        csrf_state: &str,
    ) -> SarcaResult<String> {
        let entry = state
            .take_oauth_state(csrf_state)
            .await
            .ok_or(SarcaError::OAuthFailed)?;
        if entry.provider != provider {
            return Err(SarcaError::OAuthFailed);
        }

        let profile = Self::exchange_code_for_profile(provider, code, &state.config).await?;
        let (user_id, email, email_verified) =
            OAuthService::new(&state.db).link_or_create(&profile).await?;

        let exchange_code = Uuid::new_v4().to_string();
        state
            .put_oauth_exchange(exchange_code.clone(), user_id, email, email_verified)
            .await;

        let base = state.config.public_base_url.trim_end_matches('/');
        Ok(format!("{base}/oauth/callback?code={exchange_code}"))
    }

    pub async fn exchange(
        state: &AppState,
        code: &str,
    ) -> SarcaResult<TokenSchema> {
        let entry = state
            .take_oauth_exchange(code)
            .await
            .ok_or(SarcaError::InvalidToken)?;
        let auth = crate::common::jwt_manager::AuthUser::new(entry.user_id, entry.email);
        Ok(AuthService::issue_tokens(
            auth,
            entry.email_verified,
            &state.config,
        ))
    }

    async fn google_profile(code: &str, config: &Config) -> SarcaResult<OAuthProfile> {
        let client_id = config
            .oauth_google_client_id
            .as_deref()
            .ok_or(SarcaError::OAuthNotConfigured)?;
        let client_secret = config
            .oauth_google_client_secret
            .as_deref()
            .ok_or(SarcaError::OAuthNotConfigured)?;
        let base = config.public_base_url.trim_end_matches('/');
        let redirect_uri = format!("{base}/api/auth/oauth/google/callback");

        let client = reqwest::Client::new();
        let token_res = client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("code", code),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("redirect_uri", &redirect_uri),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await
            .map_err(|e| {
                tracing::error!("google token exchange: {e}");
                SarcaError::OAuthFailed
            })?;

        if !token_res.status().is_success() {
            let body = token_res.text().await.unwrap_or_default();
            tracing::error!("google token error: {body}");
            return Err(SarcaError::OAuthFailed);
        }

        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
        }
        let token: TokenResp = token_res.json().await.map_err(|e| {
            tracing::error!("google token parse: {e}");
            SarcaError::OAuthFailed
        })?;

        let user_res = client
            .get("https://www.googleapis.com/oauth2/v3/userinfo")
            .bearer_auth(&token.access_token)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("google userinfo: {e}");
                SarcaError::OAuthFailed
            })?;

        if !user_res.status().is_success() {
            return Err(SarcaError::OAuthFailed);
        }

        #[derive(Deserialize)]
        struct GoogleUser {
            sub: String,
            email: Option<String>,
            #[serde(default)]
            email_verified: bool,
        }
        let gu: GoogleUser = user_res.json().await.map_err(|e| {
            tracing::error!("google userinfo parse: {e}");
            SarcaError::OAuthFailed
        })?;
        let email = gu.email.ok_or(SarcaError::OAuthFailed)?;

        Ok(OAuthProfile {
            provider: PROVIDER_GOOGLE.to_owned(),
            provider_user_id: gu.sub,
            email,
            email_verified: gu.email_verified,
        })
    }

    async fn github_profile(code: &str, config: &Config) -> SarcaResult<OAuthProfile> {
        let client_id = config
            .oauth_github_client_id
            .as_deref()
            .ok_or(SarcaError::OAuthNotConfigured)?;
        let client_secret = config
            .oauth_github_client_secret
            .as_deref()
            .ok_or(SarcaError::OAuthNotConfigured)?;
        let base = config.public_base_url.trim_end_matches('/');
        let redirect_uri = format!("{base}/api/auth/oauth/github/callback");

        let client = reqwest::Client::new();
        let token_res = client
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("code", code),
                ("redirect_uri", redirect_uri.as_str()),
            ])
            .send()
            .await
            .map_err(|e| {
                tracing::error!("github token exchange: {e}");
                SarcaError::OAuthFailed
            })?;

        if !token_res.status().is_success() {
            return Err(SarcaError::OAuthFailed);
        }

        #[derive(Deserialize)]
        struct TokenResp {
            access_token: Option<String>,
        }
        let token: TokenResp = token_res.json().await.map_err(|e| {
            tracing::error!("github token parse: {e}");
            SarcaError::OAuthFailed
        })?;
        let access_token = token.access_token.ok_or(SarcaError::OAuthFailed)?;

        let user_res = client
            .get("https://api.github.com/user")
            .header("User-Agent", "sarca")
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("github user: {e}");
                SarcaError::OAuthFailed
            })?;

        if !user_res.status().is_success() {
            return Err(SarcaError::OAuthFailed);
        }

        #[derive(Deserialize)]
        struct GhUser {
            id: i64,
            email: Option<String>,
        }
        let gu: GhUser = user_res.json().await.map_err(|e| {
            tracing::error!("github user parse: {e}");
            SarcaError::OAuthFailed
        })?;

        let mut email = gu.email.filter(|e| !e.is_empty());
        let mut email_verified = email.is_some();

        if email.is_none() {
            let emails_res = client
                .get("https://api.github.com/user/emails")
                .header("User-Agent", "sarca")
                .header("Accept", "application/vnd.github+json")
                .bearer_auth(&access_token)
                .send()
                .await
                .map_err(|e| {
                    tracing::error!("github emails: {e}");
                    SarcaError::OAuthFailed
                })?;

            if emails_res.status().is_success() {
                #[derive(Deserialize)]
                struct GhEmail {
                    email: String,
                    primary: bool,
                    verified: bool,
                }
                let emails: Vec<GhEmail> = emails_res.json().await.unwrap_or_default();
                if let Some(primary) = emails.iter().find(|e| e.primary && e.verified) {
                    email = Some(primary.email.clone());
                    email_verified = true;
                } else if let Some(any) = emails.iter().find(|e| e.verified) {
                    email = Some(any.email.clone());
                    email_verified = true;
                } else if let Some(first) = emails.first() {
                    email = Some(first.email.clone());
                    email_verified = first.verified;
                }
            }
        }

        let email = email.ok_or(SarcaError::OAuthFailed)?;

        Ok(OAuthProfile {
            provider: PROVIDER_GITHUB.to_owned(),
            provider_user_id: gu.id.to_string(),
            email,
            email_verified,
        })
    }
}
