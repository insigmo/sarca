use std::sync::Arc;

use axum::{
    Extension,
    Json,
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, State},
    http::{StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    common::{
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
    },
    errors::SarcaError,
    schemas::settings::{BackupPasswordSchema, RestoreResultSchema, TrashSettingsSchema},
    services::{
        backup::{BACKUP_EXTENSION, BackupService, ScratchFile, scratch_dir},
        settings::SettingsService,
    },
};

pub struct SettingsRouter;

impl SettingsRouter {
    /// Ceiling on an uploaded archive. The metadata database compresses hard,
    /// so this is orders of magnitude above a real backup — it exists to stop a
    /// bad or hostile upload from filling `WORK_DIR`, not to fit one.
    const MAX_RESTORE_BYTES: usize = 2 * 1024 * 1024 * 1024;

    pub fn get_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/trash", get(Self::get_trash).put(Self::set_trash))
            .route("/backup", post(Self::create_backup))
            .route(
                "/restore",
                post(Self::restore_backup).layer(DefaultBodyLimit::max(Self::MAX_RESTORE_BYTES)),
            )
            .route_layer(middleware::from_fn_with_state(state.clone(), logged_in_required))
            .with_state(state)
    }

    fn service(state: &AppState) -> SettingsService<'_> {
        SettingsService::new(&state.db)
    }

    /// Settings on this screen are global, so only the superuser may change
    /// them: any other account could otherwise reconfigure — or walk off with —
    /// the whole instance.
    fn require_superuser(state: &AppState, user: &AuthUser) -> Result<(), (StatusCode, String)> {
        if user.email.eq_ignore_ascii_case(&state.config.superuser_email) {
            Ok(())
        } else {
            Err(<(StatusCode, String)>::from(SarcaError::Forbidden))
        }
    }

    async fn get_trash(
        State(state): State<Arc<AppState>>,
        Extension(_user): Extension<AuthUser>,
    ) -> Result<Json<TrashSettingsSchema>, (StatusCode, String)> {
        Self::service(&state).get_trash().await.map(Json).map_err(Into::into)
    }

    /// Trash retention is global: only the superuser may shorten it, or any account
    /// could force an early purge of everyone else's trashed files.
    async fn set_trash(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        Json(body): Json<TrashSettingsSchema>,
    ) -> Result<Json<TrashSettingsSchema>, (StatusCode, String)> {
        Self::require_superuser(&state, &user)?;
        Self::service(&state).set_trash(body.retention_days).await.map(Json).map_err(Into::into)
    }

    /// Download a `.sarcabak` archive of the metadata database.
    ///
    /// POST, not GET: the optional password belongs in a request body, never in
    /// a URL that lands in proxy logs and browser history.
    async fn create_backup(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        body: Option<Json<BackupPasswordSchema>>,
    ) -> Result<Response, (StatusCode, String)> {
        Self::require_superuser(&state, &user)?;

        let password = body
            .and_then(|Json(body)| body.password)
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty());

        let artifact =
            BackupService::create(&state.db, &state.config.work_dir, password.as_deref())
                .await
                .map_err(<(StatusCode, String)>::from)?;

        let file = tokio::fs::File::open(artifact.file.path()).await.map_err(|e| {
            tracing::error!("opening finished backup failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong".to_owned())
        })?;

        // The guard moves into the stream, so the archive is removed once the
        // download finishes — and just as importantly when the client hangs up
        // halfway through it.
        let guard = artifact.file;
        let stream = async_stream::stream! {
            let _guard = guard;
            let mut reader = ReaderStream::new(file);
            while let Some(chunk) = reader.next().await {
                yield chunk;
            }
        };

        Ok((
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", artifact.filename),
                ),
                (header::CONTENT_LENGTH, artifact.size_bytes.to_string()),
                (header::CACHE_CONTROL, "no-store".to_owned()),
            ],
            Body::from_stream(stream),
        )
            .into_response())
    }

    /// Replace this instance's database with the contents of an uploaded
    /// archive. Destructive by design — see `services::backup`.
    async fn restore_backup(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        mut multipart: Multipart,
    ) -> Result<Json<RestoreResultSchema>, (StatusCode, String)> {
        Self::require_superuser(&state, &user)?;

        let dir =
            scratch_dir(&state.config.work_dir).await.map_err(<(StatusCode, String)>::from)?;
        let upload =
            ScratchFile::new(dir.join(format!("upload-{}.{BACKUP_EXTENSION}", Uuid::new_v4())));

        let mut file = tokio::fs::File::create(upload.path()).await.map_err(|e| {
            tracing::error!("creating restore upload file failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Can't create temp file".to_owned())
        })?;

        let mut password: Option<String> = None;
        let mut received = 0u64;

        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid multipart".to_owned()))?
        {
            match field.name().unwrap_or("") {
                "file" => {
                    while let Some(chunk) = field
                        .chunk()
                        .await
                        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid file stream".to_owned()))?
                    {
                        received += chunk.len() as u64;
                        file.write_all(&chunk).await.map_err(|e| {
                            tracing::error!("writing restore upload failed: {e}");
                            (StatusCode::INTERNAL_SERVER_ERROR, "Can't write temp file".to_owned())
                        })?;
                    }
                },
                "password" => {
                    let raw = field
                        .text()
                        .await
                        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid password".to_owned()))?;
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        password = Some(trimmed.to_owned());
                    }
                },
                _ => {},
            }
        }

        file.flush().await.ok();
        drop(file);

        if received == 0 {
            return Err((StatusCode::BAD_REQUEST, "No backup file was uploaded".to_owned()));
        }

        let result =
            BackupService::restore(&state.db, &state.config, upload.path(), password.as_deref())
                .await
                .map_err(<(StatusCode, String)>::from)?;

        Ok(Json(result))
    }
}
