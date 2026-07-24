use std::sync::Arc;

use axum::{
    Extension,
    Json,
    Router,
    extract::{Path, Query, State},
    middleware,
    response::{IntoResponse, Redirect},
    routing::{get, post},
};
use reqwest::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    common::{
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
    },
    models::oauth_accounts::{PROVIDER_GITHUB, PROVIDER_GOOGLE},
    schemas::auth::{
        ForgotPasswordSchema,
        LoginSchema,
        MeSchema,
        OAuthExchangeSchema,
        ProvidersSchema,
        RefreshSchema,
        ResetPasswordSchema,
        TokenBodySchema,
        TokenSchema,
    },
    services::{auth::AuthService, oauth::OAuthService},
};

pub struct AuthRouter;

impl AuthRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        let protected = Router::new()
            .route("/me", get(Self::me))
            .route("/verify/request", post(Self::verify_request))
            .route_layer(middleware::from_fn_with_state(state.clone(), logged_in_required));

        Router::new()
            .route("/login", post(Self::login))
            .route("/refresh", post(Self::refresh))
            .route("/providers", get(Self::providers))
            .route("/verify", post(Self::verify).get(Self::verify_get))
            .route("/password/forgot", post(Self::forgot_password))
            .route("/password/reset", post(Self::reset_password))
            .route("/oauth/:provider/start", get(Self::oauth_start))
            .route("/oauth/:provider/callback", get(Self::oauth_callback))
            .route("/oauth/exchange", post(Self::oauth_exchange))
            .merge(protected)
            .with_state(state)
    }

    async fn login(
        State(state): State<Arc<AppState>>,
        Json(login_data): Json<LoginSchema>,
    ) -> impl IntoResponse {
        let schema = AuthService::new(&state.db).login(login_data, &state.config).await?;

        Ok::<_, (StatusCode, String)>((StatusCode::OK, Json(schema)))
    }

    async fn refresh(
        State(state): State<Arc<AppState>>,
        Json(body): Json<RefreshSchema>,
    ) -> Result<(StatusCode, Json<TokenSchema>), (StatusCode, String)> {
        let schema =
            AuthService::new(&state.db).refresh(&body.refresh_token, &state.config).await?;

        Ok((StatusCode::OK, Json(schema)))
    }

    async fn me(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
    ) -> Result<Json<MeSchema>, (StatusCode, String)> {
        AuthService::new(&state.db).me(&user).await.map(Json).map_err(Into::into)
    }

    async fn providers(State(state): State<Arc<AppState>>) -> Json<ProvidersSchema> {
        Json(AuthService::providers(&state.config))
    }

    async fn verify_request(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        AuthService::new(&state.db).request_verify(&user, &state.config).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn verify(
        State(state): State<Arc<AppState>>,
        Json(body): Json<TokenBodySchema>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        AuthService::new(&state.db).verify_token(&body.token).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn verify_get(
        State(state): State<Arc<AppState>>,
        Query(q): Query<TokenQuery>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        AuthService::new(&state.db).verify_token(&q.token).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn forgot_password(
        State(state): State<Arc<AppState>>,
        Json(body): Json<ForgotPasswordSchema>,
    ) -> StatusCode {
        AuthService::new(&state.db).forgot_password(&body.email, &state.config).await;
        StatusCode::NO_CONTENT
    }

    async fn reset_password(
        State(state): State<Arc<AppState>>,
        Json(body): Json<ResetPasswordSchema>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        AuthService::new(&state.db).reset_password(&body.token, &body.new_password).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn oauth_start(
        State(state): State<Arc<AppState>>,
        Path(provider): Path<String>,
    ) -> Result<Redirect, (StatusCode, String)> {
        let provider = normalize_provider(&provider)?;
        let csrf = Uuid::new_v4().to_string();
        let url = OAuthService::authorize_url(provider, &state.config, &csrf)?;
        state.put_oauth_state(csrf, provider.to_owned()).await;
        Ok(Redirect::temporary(&url))
    }

    async fn oauth_callback(
        State(state): State<Arc<AppState>>,
        Path(provider): Path<String>,
        Query(q): Query<OAuthCallbackQuery>,
    ) -> Result<Redirect, (StatusCode, String)> {
        let provider = normalize_provider(&provider)?;
        if q.error.is_some() || q.code.is_none() || q.state.is_none() {
            let base = state.config.public_base_url.trim_end_matches('/');
            return Ok(Redirect::temporary(&format!("{base}/login?oauth=error")));
        }
        let code = q.code.as_deref().unwrap();
        let csrf = q.state.as_deref().unwrap();
        match OAuthService::complete_login(&state, provider, code, csrf).await {
            Ok(url) => Ok(Redirect::temporary(&url)),
            Err(e) => {
                tracing::warn!("oauth callback failed: {e}");
                let base = state.config.public_base_url.trim_end_matches('/');
                Ok(Redirect::temporary(&format!("{base}/login?oauth=error")))
            },
        }
    }

    async fn oauth_exchange(
        State(state): State<Arc<AppState>>,
        Json(body): Json<OAuthExchangeSchema>,
    ) -> Result<(StatusCode, Json<TokenSchema>), (StatusCode, String)> {
        let schema = OAuthService::exchange(&state, &body.code).await?;
        Ok((StatusCode::OK, Json(schema)))
    }
}

#[derive(Deserialize)]
struct TokenQuery {
    token: String,
}

#[derive(Deserialize)]
struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn normalize_provider(provider: &str) -> Result<&'static str, (StatusCode, String)> {
    match provider.to_ascii_lowercase().as_str() {
        "google" => Ok(PROVIDER_GOOGLE),
        "github" => Ok(PROVIDER_GITHUB),
        _ => Err((StatusCode::BAD_REQUEST, "unknown OAuth provider".into())),
    }
}
