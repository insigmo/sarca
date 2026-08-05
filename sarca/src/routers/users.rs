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

use crate::{
    common::{
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
    },
    schemas::{
        auth::TokenSchema,
        users::{
            ChangeOwnPassword,
            InUser,
            SetDisabled,
            SetPassword,
            UserDirectorySchema,
            UserListSchema,
        },
    },
    services::{auth::AuthService, users::UsersService},
};

pub struct UsersRouter;

impl UsersRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/", get(Self::list).post(Self::create))
            .route("/directory", get(Self::directory))
            // Registered before `/{user_id}/…` so `me` is not swallowed by the
            // Uuid path extractor.
            .route("/me/password", axum::routing::put(Self::change_own_password))
            .route("/{user_id}", axum::routing::delete(Self::delete))
            .route("/{user_id}/password", axum::routing::put(Self::set_password))
            .route("/{user_id}/disabled", axum::routing::put(Self::set_disabled))
            .route_layer(middleware::from_fn_with_state(state.clone(), logged_in_required))
            .with_state(state)
    }

    async fn list(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
    ) -> Result<Json<UserListSchema>, (StatusCode, String)> {
        let users = UsersService::new(&state.db).list_for_superuser(&user, &state.config).await?;
        Ok(Json(UserListSchema {
            users,
        }))
    }

    async fn create(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Json(in_user): Json<InUser>,
    ) -> impl IntoResponse {
        UsersService::new(&state.db).create_by_superuser(&user, in_user, &state.config).await?;
        Ok::<_, (StatusCode, String)>(StatusCode::CREATED)
    }

    async fn delete(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(user_id): Path<Uuid>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        UsersService::new(&state.db)
            .delete_by_superuser(&user, user_id, &state.config)
            .await
            .map_err(<(StatusCode, String)>::from)?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn change_own_password(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Json(body): Json<ChangeOwnPassword>,
    ) -> Result<Json<TokenSchema>, (StatusCode, String)> {
        let updated = UsersService::new(&state.db)
            .change_own_password(&user, body)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        // `change_own_password` just evicted the caller's own session, so the
        // response must carry a fresh token pair or the UI is logged out by
        // the very request it made.
        let email_verified = updated.email_verified();
        let auth = AuthUser::new(updated.id, updated.email);
        Ok(Json(AuthService::issue_tokens(auth, email_verified, &state.config)))
    }

    async fn set_password(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(user_id): Path<Uuid>,
        Json(body): Json<SetPassword>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        UsersService::new(&state.db)
            .set_password_by_superuser(&user, user_id, body, &state.config)
            .await
            .map_err(<(StatusCode, String)>::from)?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn set_disabled(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(user_id): Path<Uuid>,
        Json(body): Json<SetDisabled>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        UsersService::new(&state.db)
            .set_disabled_by_superuser(&user, user_id, body, &state.config)
            .await
            .map_err(<(StatusCode, String)>::from)?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn directory(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
    ) -> Result<Json<UserDirectorySchema>, (StatusCode, String)> {
        let users = UsersService::new(&state.db)
            .list_directory(&user, &state.config)
            .await
            .map_err(<(StatusCode, String)>::from)?;
        Ok(Json(UserDirectorySchema {
            users,
        }))
    }
}
