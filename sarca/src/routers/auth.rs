use std::sync::Arc;

use axum::{
    Extension,
    Json,
    Router,
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
};

use crate::{
    common::{
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
        throttle::keys,
    },
    schemas::auth::{LoginSchema, MeSchema, RefreshSchema, TokenSchema},
    services::auth::AuthService,
};

pub struct AuthRouter;

impl AuthRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        let protected = Router::new()
            .route("/me", get(Self::me))
            .route("/logout", post(Self::logout))
            .route_layer(middleware::from_fn_with_state(state.clone(), logged_in_required));

        Router::new()
            .route("/login", post(Self::login))
            .route("/refresh", post(Self::refresh))
            .merge(protected)
            .with_state(state)
    }

    /// Password guessing is throttled per target address: a few free tries,
    /// then a growing delay, then 429. Without this, bcrypt is the only cost
    /// an attacker pays and the endpoint is an open password oracle.
    async fn login(
        State(state): State<Arc<AppState>>,
        Json(login_data): Json<LoginSchema>,
    ) -> Result<(StatusCode, Json<TokenSchema>), (StatusCode, String)> {
        let key = keys::login(&login_data.email);
        state.throttle.check(&key).await?;

        match AuthService::new(&state.db).login(login_data, &state.config).await {
            Ok(schema) => {
                state.throttle.record_success(&key);
                Ok((StatusCode::OK, Json(schema)))
            },
            Err(e) => {
                state.throttle.record_failure(&key);
                Err(e.into())
            },
        }
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
        AuthService::new(&state.db).me(&user, &state.config).await.map(Json).map_err(Into::into)
    }

    /// Revoke every token issued for the caller so far.
    async fn logout(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        AuthService::new(&state.db).logout(&user).await?;
        Ok(StatusCode::NO_CONTENT)
    }
}
