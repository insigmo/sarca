use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::get,
    Extension, Json, Router,
};
use uuid::Uuid;

use crate::{
    common::{
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
    },
    models::storages::Storage,
    schemas::{
        access::{GrantAccess, RestrictAccess},
        storages::{
            AddChannelSchema, InStorageSchema, StoragesListSchema, UpdateChannelSchema,
            UpdateStorageSchema,
        },
    },
    services::storages::StoragesService,
};

use super::files::FilesRouter;

pub struct StoragesRouter;

impl StoragesRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        let files_router = FilesRouter::get_router(state.clone());
        Router::new()
            .route("/", get(Self::list).post(Self::create))
            .route(
                "/:storage_id",
                get(Self::get).put(Self::update).delete(Self::delete),
            )
            .route(
                "/:storage_id/access",
                get(Self::list_users_with_access)
                    .post(Self::grant_access)
                    .delete(Self::restrict_access),
            )
            .route(
                "/:storage_id/channels",
                axum::routing::post(Self::add_channel),
            )
            .route(
                "/:storage_id/channels/:channel_id",
                axum::routing::put(Self::update_channel).delete(Self::remove_channel),
            )
            .route(
                "/:storage_id/replication/retry",
                axum::routing::post(Self::retry_replication),
            )
            .nest("/:storage_id/files", files_router)
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                logged_in_required,
            ))
            .with_state(state)
    }

    fn service<'d>(state: &'d AppState) -> StoragesService<'d> {
        StoragesService::new(
            &state.db,
            &state.config.telegram_api_base_url,
            state.config.telegram_rate_limit,
        )
    }

    async fn create(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Json(in_schema): Json<InStorageSchema>,
    ) -> impl IntoResponse {
        let storage = Self::service(&state).create(in_schema, &user).await?;
        Ok::<_, (StatusCode, String)>((StatusCode::CREATED, Json(storage)))
    }

    async fn list(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
    ) -> impl IntoResponse {
        let storages = Self::service(&state)
            .list(&user)
            .await
            .map(|s| StoragesListSchema::new(s))?;
        tracing::debug!(
            "[STORAGES ROUTER] Returning {} storages to client",
            storages.storages.len()
        );
        Ok::<_, (StatusCode, String)>(Json(storages))
    }

    async fn get(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
    ) -> impl IntoResponse {
        let storage = Self::service(&state).get_detail(id, &user).await?;
        Ok::<_, (StatusCode, String)>(Json(storage))
    }

    async fn update(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
        Json(in_schema): Json<UpdateStorageSchema>,
    ) -> Result<Json<Storage>, (StatusCode, String)> {
        let storage = Self::service(&state).update(id, in_schema, &user).await?;
        Ok(Json(storage))
    }

    async fn delete(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state).delete(id, &user).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn add_channel(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
        Json(in_schema): Json<AddChannelSchema>,
    ) -> impl IntoResponse {
        let channel = Self::service(&state)
            .add_channel(storage_id, in_schema, &user)
            .await?;
        Ok::<_, (StatusCode, String)>((StatusCode::CREATED, Json(channel)))
    }

    async fn update_channel(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path((storage_id, channel_id)): Path<(Uuid, Uuid)>,
        Json(patch): Json<UpdateChannelSchema>,
    ) -> impl IntoResponse {
        let channel = Self::service(&state)
            .update_channel(storage_id, channel_id, patch, &user)
            .await?;
        Ok::<_, (StatusCode, String)>(Json(channel))
    }

    async fn remove_channel(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path((storage_id, channel_id)): Path<(Uuid, Uuid)>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state)
            .remove_channel(storage_id, channel_id, &user)
            .await?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn retry_replication(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
    ) -> impl IntoResponse {
        let stats = Self::service(&state)
            .retry_replication(storage_id, &user)
            .await?;
        Ok::<_, (StatusCode, String)>(Json(stats))
    }

    async fn grant_access(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
        Json(in_schema): Json<GrantAccess>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state).grant_access(id, in_schema, &user).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn list_users_with_access(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
    ) -> impl IntoResponse {
        let users = Self::service(&state)
            .list_users_with_access(id, &user)
            .await?;
        Ok::<_, (StatusCode, String)>(Json(users))
    }

    async fn restrict_access(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
        Json(in_schema): Json<RestrictAccess>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state)
            .restrict_access(id, in_schema, &user)
            .await?;
        Ok(StatusCode::NO_CONTENT)
    }
}
