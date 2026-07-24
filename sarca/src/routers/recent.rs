use std::sync::Arc;

use axum::{
    Extension,
    Json,
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use uuid::Uuid;

use crate::{
    common::{jwt_manager::AuthUser, routing::app_state::AppState},
    schemas::recent::RecentPathSchema,
    services::recent::RecentService,
};

pub struct RecentRouter;

impl RecentRouter {
    pub fn get_router(state: Arc<AppState>) -> Router<Arc<AppState>, axum::body::Body> {
        Router::new().route("/", get(Self::list).post(Self::record)).with_state(state)
    }

    fn service(state: &AppState) -> RecentService<'_> {
        RecentService::new(&state.db)
    }

    async fn list(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
    ) -> Result<impl IntoResponse, (StatusCode, String)> {
        Self::service(&state).list(storage_id, &user).await.map(Json).map_err(Into::into)
    }

    async fn record(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
        Json(body): Json<RecentPathSchema>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state)
            .record(storage_id, &body.path, &user)
            .await
            .map(|()| StatusCode::NO_CONTENT)
            .map_err(Into::into)
    }
}
