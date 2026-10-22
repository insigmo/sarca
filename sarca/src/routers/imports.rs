use std::sync::Arc;

use axum::{
    Extension,
    Json,
    Router,
    extract::{Path, State},
    http::StatusCode,
    middleware,
    routing::{get, post},
};
use uuid::Uuid;

use crate::{
    common::{
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
    },
    errors::SarcaError,
    schemas::imports::{
        CreateImportSchema,
        ImportDetailSchema,
        ImportJobSchema,
        ImportListSchema,
        ResumeImportSchema,
    },
    services::import::{ImportService, NewImport},
};

type ApiResult<T> = Result<T, (StatusCode, String)>;

pub struct ImportsRouter;

impl ImportsRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/", get(Self::list).post(Self::create))
            .route("/{id}", get(Self::get).delete(Self::delete))
            .route("/{id}/pause", post(Self::pause))
            .route("/{id}/resume", post(Self::resume))
            .route("/{id}/retry", post(Self::retry))
            .route("/{id}/cancel", post(Self::cancel))
            .route_layer(middleware::from_fn_with_state(state.clone(), logged_in_required))
            .with_state(state)
    }

    /// Superuser-only for now: an import makes this server fetch an address the
    /// caller chooses, which is a request-forgery lever no ordinary account
    /// should hold.
    fn require_superuser(state: &AppState, user: &AuthUser) -> ApiResult<()> {
        if user.email.eq_ignore_ascii_case(&state.config.superuser_email) {
            Ok(())
        } else {
            Err(SarcaError::Forbidden.into())
        }
    }

    async fn list(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
    ) -> ApiResult<Json<ImportListSchema>> {
        Self::require_superuser(&state, &user)?;
        let jobs = ImportService::new(&state).list().await?;
        Ok(Json(ImportListSchema {
            imports: jobs.into_iter().map(Into::into).collect(),
        }))
    }

    async fn get(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
    ) -> ApiResult<Json<ImportDetailSchema>> {
        Self::require_superuser(&state, &user)?;
        let (job, failed) = ImportService::new(&state).get(id).await?;
        Ok(Json(ImportDetailSchema {
            job: job.into(),
            failed_items: failed.into_iter().map(Into::into).collect(),
        }))
    }

    async fn create(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Json(body): Json<CreateImportSchema>,
    ) -> ApiResult<(StatusCode, Json<ImportJobSchema>)> {
        Self::require_superuser(&state, &user)?;
        let job = ImportService::new(&state)
            .create(
                &user,
                NewImport {
                    storage_id: body.storage_id,
                    url: body.url,
                    username: body.username,
                    password: body.password,
                    target_path: body.target_path,
                },
            )
            .await?;
        Ok((StatusCode::CREATED, Json(job.into())))
    }

    async fn pause(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
    ) -> ApiResult<Json<ImportJobSchema>> {
        Self::require_superuser(&state, &user)?;
        Ok(Json(ImportService::new(&state).pause(id).await?.into()))
    }

    async fn resume(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
        body: Option<Json<ResumeImportSchema>>,
    ) -> ApiResult<Json<ImportJobSchema>> {
        Self::require_superuser(&state, &user)?;
        let body = body.map(|Json(b)| b).unwrap_or_default();
        Ok(Json(ImportService::new(&state).resume(id, body.password.as_deref()).await?.into()))
    }

    async fn retry(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
    ) -> ApiResult<Json<ImportJobSchema>> {
        Self::require_superuser(&state, &user)?;
        Ok(Json(ImportService::new(&state).retry_failed(id).await?.into()))
    }

    async fn cancel(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
    ) -> ApiResult<Json<ImportJobSchema>> {
        Self::require_superuser(&state, &user)?;
        Ok(Json(ImportService::new(&state).cancel(id).await?.into()))
    }

    async fn delete(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Path(id): Path<Uuid>,
    ) -> ApiResult<StatusCode> {
        Self::require_superuser(&state, &user)?;
        ImportService::new(&state).delete(id).await?;
        Ok(StatusCode::NO_CONTENT)
    }
}
