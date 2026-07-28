use std::sync::Arc;

use axum::{
    Extension,
    Json,
    Router,
    extract::State,
    middleware,
    response::IntoResponse,
    routing::get,
};
use reqwest::StatusCode;

use crate::{
    common::{
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
    },
    schemas::users::{InUser, UserListSchema},
    services::users::UsersService,
};

pub struct UsersRouter;

impl UsersRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/", get(Self::list).post(Self::create))
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
}
