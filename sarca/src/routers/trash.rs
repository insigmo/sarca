use std::sync::Arc;

use axum::{
    Extension,
    Json,
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use uuid::Uuid;

use crate::{
    common::{jwt_manager::AuthUser, routing::app_state::AppState},
    schemas::files::{RestoreTrashSchema, TrashListQuery},
    services::trash::TrashService,
};

pub struct TrashRouter;

impl TrashRouter {
    pub fn get_router(state: Arc<AppState>) -> Router<Arc<AppState>, axum::body::Body> {
        Router::new()
            .route("/", get(Self::list).delete(Self::empty))
            .route("/restore", post(Self::restore))
            .route("/*path", delete(Self::delete_forever))
            .with_state(state)
    }

    fn service(state: &AppState) -> TrashService<'_> {
        TrashService::new(
            &state.db,
            &state.config.telegram_api_base_url,
            state.config.telegram_rate_limit,
        )
    }

    async fn list(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
        Query(query): Query<TrashListQuery>,
    ) -> Result<impl IntoResponse, (StatusCode, String)> {
        let path = query.path.as_deref().unwrap_or("");
        Self::service(&state).list(storage_id, path, &user).await.map(Json).map_err(Into::into)
    }

    async fn restore(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
        Json(body): Json<RestoreTrashSchema>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state)
            .restore(storage_id, &body.path, body.on_conflict.as_deref(), &user)
            .await
            .map(|()| StatusCode::NO_CONTENT)
            .map_err(Into::into)
    }

    async fn empty(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state)
            .empty(storage_id, &user)
            .await
            .map(|()| StatusCode::NO_CONTENT)
            .map_err(Into::into)
    }

    async fn delete_forever(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path((storage_id, path)): Path<(Uuid, String)>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        let path = percent_encoding::percent_decode_str(&path).decode_utf8_lossy().to_string();
        Self::service(&state)
            .delete_forever(storage_id, &path, &user)
            .await
            .map(|()| StatusCode::NO_CONTENT)
            .map_err(Into::into)
    }
}
