use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
    Extension, Json, Router,
};

use crate::{
    common::{
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
    },
    schemas::setup::{
        BotTokenSchema, ChannelPollSchema, LocalApiCredentialsSchema, SetupCreateStorageSchema,
    },
    services::setup::SetupService,
};

pub struct SetupRouter;

impl SetupRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/status", get(Self::status))
            .route("/local-api", post(Self::save_local_api))
            .route("/local-api/verify", post(Self::verify_local_api))
            .route("/local-api/skip", post(Self::skip_local_api))
            .route("/bot/validate", post(Self::validate_bot))
            .route("/channel/poll", post(Self::poll_channel))
            .route("/storages", post(Self::create_storage))
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                logged_in_required,
            ))
            .with_state(state)
    }

    fn service<'d>(state: &'d AppState) -> SetupService<'d> {
        SetupService::new(
            &state.db,
            &state.config.telegram_api_base_url,
            state.config.telegram_rate_limit,
        )
    }

    async fn status(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
    ) -> Result<Json<crate::schemas::setup::SetupStatusSchema>, (StatusCode, String)> {
        Self::service(&state)
            .status(&user)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    async fn save_local_api(
        State(state): State<Arc<AppState>>,
        Extension(_user): Extension<AuthUser>,
        Json(body): Json<LocalApiCredentialsSchema>,
    ) -> Result<Json<crate::schemas::setup::LocalApiSaveResultSchema>, (StatusCode, String)> {
        Self::service(&state)
            .save_local_api(body)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    async fn verify_local_api(
        State(state): State<Arc<AppState>>,
        Extension(_user): Extension<AuthUser>,
    ) -> Result<Json<crate::schemas::setup::LocalApiVerifySchema>, (StatusCode, String)> {
        Self::service(&state)
            .verify_local_api()
            .await
            .map(Json)
            .map_err(Into::into)
    }

    async fn skip_local_api(
        State(state): State<Arc<AppState>>,
        Extension(_user): Extension<AuthUser>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state)
            .skip_local_api()
            .await
            .map(|_| StatusCode::NO_CONTENT)
            .map_err(Into::into)
    }

    async fn validate_bot(
        State(state): State<Arc<AppState>>,
        Extension(_user): Extension<AuthUser>,
        Json(body): Json<BotTokenSchema>,
    ) -> Result<Json<crate::schemas::setup::BotValidateSchema>, (StatusCode, String)> {
        Self::service(&state)
            .validate_bot(&body.token)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    async fn poll_channel(
        State(state): State<Arc<AppState>>,
        Extension(_user): Extension<AuthUser>,
        Json(body): Json<ChannelPollSchema>,
    ) -> Result<Json<crate::schemas::setup::ChannelPollResultSchema>, (StatusCode, String)> {
        Self::service(&state)
            .poll_channel(&body.token, &body.exclude_chat_ids)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    async fn create_storage(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Json(body): Json<SetupCreateStorageSchema>,
    ) -> Result<Json<crate::schemas::setup::SetupCreateStorageResultSchema>, (StatusCode, String)>
    {
        Self::service(&state)
            .create_storage(body, &user)
            .await
            .map(Json)
            .map_err(Into::into)
    }
}
