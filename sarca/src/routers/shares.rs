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
    schemas::shares::CreateShareSchema,
    services::shares::SharesService,
};

pub struct SharesRouter;

impl SharesRouter {
    pub fn get_router(state: Arc<AppState>) -> Router<Arc<AppState>, axum::body::Body> {
        Router::new()
            .route("/", get(Self::list).post(Self::create))
            .route("/:share_id", delete(Self::revoke))
            .with_state(state)
    }

    fn service(state: &AppState) -> SharesService<'_> {
        SharesService::new(&state.db)
    }

    async fn create(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
        Json(body): Json<CreateShareSchema>,
    ) -> Result<impl IntoResponse, (StatusCode, String)> {
        let link = Self::service(&state)
            .create(storage_id, &body.path, body.expires_at, body.password.as_deref(), &user)
            .await?;
        Ok((StatusCode::CREATED, Json(link)))
    }

    async fn list(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
    ) -> Result<impl IntoResponse, (StatusCode, String)> {
        Self::service(&state).list(storage_id, &user).await.map(Json).map_err(Into::into)
    }

    async fn revoke(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path((storage_id, share_id)): Path<(Uuid, Uuid)>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state)
            .revoke(storage_id, share_id, &user)
            .await
            .map(|()| StatusCode::NO_CONTENT)
            .map_err(Into::into)
    }
}
