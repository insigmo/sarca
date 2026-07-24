use std::sync::Arc;

use axum::{
    Extension,
    Json,
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get},
};
use uuid::Uuid;

use crate::{
    common::{jwt_manager::AuthUser, routing::app_state::AppState},
    schemas::favorites::FavoritePathSchema,
    services::favorites::FavoritesService,
};

pub struct FavoritesRouter;

impl FavoritesRouter {
    pub fn get_router(state: Arc<AppState>) -> Router<Arc<AppState>, axum::body::Body> {
        Router::new()
            .route("/", get(Self::list).put(Self::add))
            .route("/*path", delete(Self::remove))
            .with_state(state)
    }

    fn service(state: &AppState) -> FavoritesService<'_> {
        FavoritesService::new(&state.db)
    }

    async fn list(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
    ) -> Result<impl IntoResponse, (StatusCode, String)> {
        Self::service(&state).list(storage_id, &user).await.map(Json).map_err(Into::into)
    }

    async fn add(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
        Json(body): Json<FavoritePathSchema>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state)
            .add(storage_id, &body.path, &user)
            .await
            .map(|()| StatusCode::NO_CONTENT)
            .map_err(Into::into)
    }

    async fn remove(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path((storage_id, path)): Path<(Uuid, String)>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        let path = percent_encoding::percent_decode_str(&path).decode_utf8_lossy().to_string();
        Self::service(&state)
            .remove(storage_id, &path, &user)
            .await
            .map(|()| StatusCode::NO_CONTENT)
            .map_err(Into::into)
    }
}
