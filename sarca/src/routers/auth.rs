use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
use reqwest::StatusCode;

use crate::{
    common::routing::app_state::AppState,
    schemas::auth::{LoginSchema, RefreshSchema, TokenSchema},
    services::auth::AuthService,
};

pub struct AuthRouter;

impl AuthRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/login", post(Self::login))
            .route("/refresh", post(Self::refresh))
            .with_state(state)
    }

    async fn login(
        State(state): State<Arc<AppState>>,
        Json(login_data): Json<LoginSchema>,
    ) -> impl IntoResponse {
        let schema = AuthService::new(&state.db)
            .login(login_data, &state.config)
            .await?;

        Ok::<_, (StatusCode, String)>((StatusCode::OK, Json(schema)))
    }

    async fn refresh(
        State(state): State<Arc<AppState>>,
        Json(body): Json<RefreshSchema>,
    ) -> Result<(StatusCode, Json<TokenSchema>), (StatusCode, String)> {
        let schema = AuthService::new(&state.db)
            .refresh(&body.refresh_token, &state.config)
            .await?;

        Ok((StatusCode::OK, Json(schema)))
    }
}
