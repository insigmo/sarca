use std::sync::Arc;

use axum::{Extension, Json, Router, extract::State, http::StatusCode, middleware, routing::get};

use crate::{
    common::{
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
    },
    schemas::settings::TrashSettingsSchema,
    services::settings::SettingsService,
};

pub struct SettingsRouter;

impl SettingsRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/trash", get(Self::get_trash).put(Self::set_trash))
            .route_layer(middleware::from_fn_with_state(state.clone(), logged_in_required))
            .with_state(state)
    }

    fn service(state: &AppState) -> SettingsService<'_> {
        SettingsService::new(&state.db)
    }

    async fn get_trash(
        State(state): State<Arc<AppState>>,
        Extension(_user): Extension<AuthUser>,
    ) -> Result<Json<TrashSettingsSchema>, (StatusCode, String)> {
        Self::service(&state).get_trash().await.map(Json).map_err(Into::into)
    }

    async fn set_trash(
        State(state): State<Arc<AppState>>,
        Extension(_user): Extension<AuthUser>,
        Json(body): Json<TrashSettingsSchema>,
    ) -> Result<Json<TrashSettingsSchema>, (StatusCode, String)> {
        Self::service(&state).set_trash(body.retention_days).await.map(Json).map_err(Into::into)
    }
}
