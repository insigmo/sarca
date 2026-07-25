use std::sync::Arc;

use axum::{
    Extension,
    Json,
    Router,
    extract::{Path, Query, State},
    routing::get,
};
use uuid::Uuid;

use crate::{
    common::{jwt_manager::AuthUser, routing::app_state::AppState},
    schemas::sync::ChangelogQuery,
    services::sync::SyncService,
};

pub struct SyncRouter;

impl SyncRouter {
    pub fn get_router(state: Arc<AppState>) -> Router<Arc<AppState>, axum::body::Body> {
        Router::new()
            .route("/changelog", get(Self::changelog))
            .route("/snapshot", get(Self::snapshot))
            .with_state(state)
    }

    async fn changelog(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
        Query(query): Query<ChangelogQuery>,
    ) -> Result<Json<crate::schemas::sync::ChangelogResponse>, (axum::http::StatusCode, String)>
    {
        let resp = SyncService::new(&state.db).changelog(storage_id, &user, query).await?;
        Ok(Json(resp))
    }

    async fn snapshot(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
    ) -> Result<Json<crate::schemas::sync::SnapshotResponse>, (axum::http::StatusCode, String)>
    {
        let resp = SyncService::new(&state.db).snapshot(storage_id, &user).await?;
        Ok(Json(resp))
    }
}
