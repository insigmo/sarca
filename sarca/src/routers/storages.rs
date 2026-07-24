use std::sync::Arc;

use axum::{
    Extension,
    Json,
    Router,
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::get,
};
use uuid::Uuid;

use super::{
    favorites::FavoritesRouter,
    files::FilesRouter,
    recent::RecentRouter,
    shares::SharesRouter,
    trash::TrashRouter,
};
use crate::{
    common::{
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
    },
    models::storages::Storage,
    schemas::{
        access::{GrantAccess, RestrictAccess},
        storages::{
            AddChannelSchema,
            InStorageSchema,
            RefreshChannelsResultSchema,
            SetStorageBotSchema,
            StoragesListSchema,
            UpdateChannelSchema,
            UpdateStorageSchema,
        },
    },
    services::storages::StoragesService,
};

pub struct StoragesRouter;

impl StoragesRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        let files_router = FilesRouter::get_router(state.clone());
        let trash_router = TrashRouter::get_router(state.clone());
        let favorites_router = FavoritesRouter::get_router(state.clone());
        let recent_router = RecentRouter::get_router(state.clone());
        let shares_router = SharesRouter::get_router(state.clone());
        Router::new()
            .route("/", get(Self::list).post(Self::create))
            .route("/:storage_id", get(Self::get).put(Self::update).delete(Self::delete))
            .route(
                "/:storage_id/access",
                get(Self::list_users_with_access)
                    .post(Self::grant_access)
                    .delete(Self::restrict_access),
            )
            .route("/:storage_id/channels", axum::routing::post(Self::add_channel))
            .route("/:storage_id/channels/refresh", axum::routing::post(Self::refresh_channels))
            .route("/:storage_id/bot", axum::routing::put(Self::set_bot))
            .route(
                "/:storage_id/channels/:channel_id",
                axum::routing::put(Self::update_channel).delete(Self::remove_channel),
            )
            .route("/:storage_id/replication/retry", axum::routing::post(Self::retry_replication))
            .nest("/:storage_id/files", files_router)
            .nest("/:storage_id/trash", trash_router)
            .nest("/:storage_id/favorites", favorites_router)
            .nest("/:storage_id/recent", recent_router)
            .nest("/:storage_id/shares", shares_router)
            .route_layer(middleware::from_fn_with_state(state.clone(), logged_in_required))
            .with_state(state)
    }

    fn service(state: &AppState) -> StoragesService<'_> {
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
        let storages = Self::service(&state).list(&user).await.map(StoragesListSchema::new)?;
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
        let channel = Self::service(&state).add_channel(storage_id, in_schema, &user).await?;
        Ok::<_, (StatusCode, String)>((StatusCode::CREATED, Json(channel)))
    }

    async fn refresh_channels(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
    ) -> Result<Json<RefreshChannelsResultSchema>, (StatusCode, String)> {
        let result = Self::service(&state).refresh_channels(storage_id, &user).await?;
        Ok(Json(result))
    }

    async fn set_bot(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
        Json(body): Json<SetStorageBotSchema>,
    ) -> impl IntoResponse {
        let bot = Self::service(&state).set_bot(storage_id, body, &user).await?;
        Ok::<_, (StatusCode, String)>(Json(bot))
    }

    async fn update_channel(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path((storage_id, channel_id)): Path<(Uuid, Uuid)>,
        Json(patch): Json<UpdateChannelSchema>,
    ) -> impl IntoResponse {
        let channel =
            Self::service(&state).update_channel(storage_id, channel_id, patch, &user).await?;
        Ok::<_, (StatusCode, String)>(Json(channel))
    }

    async fn remove_channel(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path((storage_id, channel_id)): Path<(Uuid, Uuid)>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state).remove_channel(storage_id, channel_id, &user).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn retry_replication(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(storage_id): Path<Uuid>,
    ) -> impl IntoResponse {
        let replication = Self::service(&state).retry_replication(storage_id, &user).await?;
        Ok::<_, (StatusCode, String)>(Json(replication))
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
        let users = Self::service(&state).list_users_with_access(id, &user).await?;
        Ok::<_, (StatusCode, String)>(Json(users))
    }

    async fn restrict_access(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
        Json(in_schema): Json<RestrictAccess>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state).restrict_access(id, in_schema, &user).await?;
        Ok(StatusCode::NO_CONTENT)
    }
}
