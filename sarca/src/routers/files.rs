use std::{
    collections::HashMap,
    io,
    path::Path,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use async_stream::try_stream;
use axum::{
    Extension,
    Json,
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Multipart, Path as RoutePath, Query, State},
    http::{HeaderMap, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures::{Stream, StreamExt};
use percent_encoding::percent_decode_str;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::mpsc,
};
use uuid::Uuid;

use crate::{
    common::{
        access::check_access,
        channels::UploadProgressEvent,
        chunk_cache::ChunkCache,
        jwt_manager::AuthUser,
        media_cache::MediaCache,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
        single_flight::SingleFlight,
        telegram_api::bot_api::{TelegramBotApi, is_chat_dead_error},
    },
    errors::{SarcaError, SarcaResult},
    models::{access::AccessType, files::InFile, storage_channels::StorageChannel},
    repositories::{
        access::AccessRepository,
        files::FilesRepository,
        storage_channels::StorageChannelsRepository,
        storages::StoragesRepository,
    },
    schemas::files::{
        CopySchema,
        InFolderSchema,
        MoveSchema,
        RenameSchema,
        SearchQuery,
        UploadParams,
    },
    services::{
        files::FilesService,
        storage_manager::StorageManagerService,
        storage_workers_scheduler::StorageWorkersScheduler,
        thumbnails,
    },
};

pub struct FilesRouter;

impl FilesRouter {
    /// Max total uncompressed size of files packed into a folder ZIP.
    const MAX_FOLDER_ZIP_BYTES: i64 = 10 * 1024 * 1024 * 1024;
    /// Thumb/preview reads give up on a saturated storage worker and return 503
    /// rather than hold the request open; uploads keep waiting indefinitely.
    const MEDIA_READ_TOKEN_BUDGET: Duration = Duration::from_secs(10);

    pub fn get_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
        Router::new()
            .route("/create_folder", post(Self::create_folder))
            .route("/upload", post(Self::upload))
            .route("/rename", post(Self::rename))
            .route("/move", post(Self::move_to))
            .route("/copy", post(Self::copy_to))
            .route("/{*path}", get(Self::dynamic_get).delete(Self::delete))
            .layer(DefaultBodyLimit::disable())
            .route_layer(middleware::from_fn_with_state(state, logged_in_required))
    }

    fn service(state: &AppState) -> FilesService<'_> {
        FilesService::new(
            &state.db,
            state.tx.clone(),
            &state.config.telegram_api_base_url,
            state.config.telegram_rate_limit,
            &state.config.work_dir,
        )
    }

    async fn dynamic_get(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath((storage_id, path)): RoutePath<(Uuid, String)>,
        query: Query<SearchQuery>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let (root_path, path) = path.split_once('/').unwrap_or((&path, ""));
        match root_path {
            "tree" => Self::tree(state, user, storage_id, path).await,
            "download" => Self::download(state, user, storage_id, path, &query.0, &headers).await,
            "thumb" => Self::thumb(state, user, storage_id, path).await,
            "preview" => Self::preview(state, user, storage_id, path).await,
            "info" => Self::file_info_inner(state, user, storage_id, path).await,
            "search" => {
                if let Some(search_path) = query.0.search_path {
                    Self::search(state, user, storage_id, path, &search_path).await
                } else {
                    Err((
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "search_path query parameter is required".to_owned(),
                    ))
                }
            },
            _ => Err((StatusCode::NOT_FOUND, "Not found".to_owned())),
        }
    }

    async fn tree(
        state: Arc<AppState>,
        user: AuthUser,
        storage_id: Uuid,
        path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        let fs_layer = Self::service(&state).list_dir(storage_id, path, &user).await?;
        Ok(Json(fs_layer).into_response())
    }

    async fn file_info_inner(
        state: Arc<AppState>,
        user: AuthUser,
        storage_id: Uuid,
        path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        let path = percent_decode_str(path).decode_utf8_lossy().to_string();
        let info = Self::service(&state)
            .info(storage_id, &path, &user)
            .await
            .map_err(<(StatusCode, String)>::from)?;
        Ok(Json(info).into_response())
    }

    async fn upload(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath(storage_id): RoutePath<Uuid>,
        mut multipart: Multipart,
    ) -> Result<Response, (StatusCode, String)> {
        // stream multipart to disk
        let upload_dir = Path::new(&state.config.work_dir).join("uploads");
        tokio::fs::create_dir_all(&upload_dir).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Can't create upload directory under WORK_DIR (check permissions): {e}"),
            )
        })?;

        let tmp_path = upload_dir.join(format!("{}.upload", Uuid::new_v4()));
        let mut tmp_file = tokio::fs::File::create(&tmp_path).await.map_err(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "Can't create temp file".to_owned())
        })?;

        let cleanup_tmp = |path: &Path| {
            let path = path.to_path_buf();
            tokio::spawn(async move {
                let _ = tokio::fs::remove_file(&path).await;
            });
        };

        let (mut filename_field, mut filename_from_file, mut parent_path, mut file_size) =
            (None::<String>, None::<String>, None::<String>, 0i64);
        let mut source_mtime = None::<chrono::DateTime<chrono::Utc>>;
        let mut source_created_at = None::<chrono::DateTime<chrono::Utc>>;
        let mut content_hash = None::<String>;
        let mut client_thumb = None::<Vec<u8>>;

        while let Some(mut field) = multipart.next_field().await.map_err(|_| {
            cleanup_tmp(&tmp_path);
            (StatusCode::BAD_REQUEST, "Invalid multipart".to_owned())
        })? {
            let name = field.name().unwrap_or("").to_owned();

            match name.as_str() {
                "file" => {
                    let raw_name = field.file_name().unwrap_or("").to_owned();
                    if !raw_name.trim().is_empty() {
                        filename_from_file = Some(raw_name);
                    }
                    while let Some(chunk) = field.chunk().await.map_err(|_| {
                        cleanup_tmp(&tmp_path);
                        (StatusCode::BAD_REQUEST, "Invalid file stream".to_owned())
                    })? {
                        file_size += chunk.len() as i64;
                        tmp_file.write_all(&chunk).await.map_err(|e| {
                            cleanup_tmp(&tmp_path);
                            let disk_full = e.raw_os_error() == Some(28) /* ENOSPC */
                                || e.to_string().to_ascii_lowercase().contains("no space");
                            if disk_full {
                                (
                                    StatusCode::INSUFFICIENT_STORAGE,
                                    "Disk full while saving upload — free space under WORK_DIR \
                                     and try again"
                                        .to_owned(),
                                )
                            } else {
                                (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "Can't write temp file".to_owned(),
                                )
                            }
                        })?;
                    }
                },
                "filename" => {
                    let raw_name = field.text().await.map_err(|_| {
                        cleanup_tmp(&tmp_path);
                        (StatusCode::BAD_REQUEST, "Invalid filename".to_owned())
                    })?;
                    let decoded = percent_decode_str(&raw_name).decode_utf8_lossy();
                    if !decoded.trim().is_empty() {
                        filename_field = Some(decoded.into_owned());
                    }
                },
                "path" => {
                    let raw_path = field.text().await.map_err(|_| {
                        cleanup_tmp(&tmp_path);
                        (StatusCode::BAD_REQUEST, "Invalid path".to_owned())
                    })?;
                    let decoded = percent_decode_str(&raw_path).decode_utf8_lossy();
                    parent_path = Some(decoded.into_owned());
                },
                "mtime" => {
                    let raw = field.text().await.map_err(|_| {
                        cleanup_tmp(&tmp_path);
                        (StatusCode::BAD_REQUEST, "Invalid mtime".to_owned())
                    })?;
                    source_mtime = Self::parse_epoch_millis(&raw);
                },
                "created" => {
                    let raw = field.text().await.map_err(|_| {
                        cleanup_tmp(&tmp_path);
                        (StatusCode::BAD_REQUEST, "Invalid created".to_owned())
                    })?;
                    source_created_at = Self::parse_epoch_millis(&raw);
                },
                "thumb" => {
                    // Grid thumbnail built by the client from the picture it is
                    // uploading. Stored verbatim; the server never decodes the
                    // original for it. Anything bigger than a 128px JPEG could
                    // plausibly be is not a thumbnail, so drop it.
                    let mut bytes = Vec::new();
                    let mut too_big = false;
                    while let Some(chunk) = field.chunk().await.map_err(|_| {
                        cleanup_tmp(&tmp_path);
                        (StatusCode::BAD_REQUEST, "Invalid thumb stream".to_owned())
                    })? {
                        bytes.extend_from_slice(&chunk);
                        if bytes.len() > MAX_CLIENT_THUMB_BYTES {
                            bytes.clear();
                            too_big = true;
                            break;
                        }
                    }
                    if too_big {
                        // Not silent: dropping this falls back to a server-side
                        // decode of the original, and when *that* also fails the
                        // file ends up with no thumbnail at all and no trace of
                        // why. `MAX_CLIENT_THUMB_BYTES` drifting behind
                        // `THUMB_MAX_EDGE` is the thing this line makes visible.
                        tracing::warn!(
                            "client thumb ignored: over {MAX_CLIENT_THUMB_BYTES} bytes"
                        );
                    } else if is_jpeg(&bytes) {
                        client_thumb = Some(bytes);
                    } else if !bytes.is_empty() {
                        tracing::warn!("client thumb ignored: not a JPEG");
                    }
                },
                "content_hash" => {
                    let raw = field.text().await.map_err(|_| {
                        cleanup_tmp(&tmp_path);
                        (StatusCode::BAD_REQUEST, "Invalid content_hash".to_owned())
                    })?;
                    let trimmed = raw.trim().to_owned();
                    if !trimmed.is_empty() {
                        content_hash = Some(trimmed);
                    }
                },
                _ => (),
            }
        }

        tmp_file.flush().await.map_err(|_| {
            cleanup_tmp(&tmp_path);
            (StatusCode::INTERNAL_SERVER_ERROR, "Can't flush temp file".to_owned())
        })?;

        let Some(parent_path) = parent_path else {
            cleanup_tmp(&tmp_path);
            return Err((StatusCode::BAD_REQUEST, "path field is required".to_owned()));
        };
        let filename =
            filename_field.or(filename_from_file).unwrap_or_else(|| "unnamed".to_owned());
        let path = match Self::construct_path(&parent_path, &filename) {
            Ok(p) => p,
            Err(e) => {
                cleanup_tmp(&tmp_path);
                return Err(<(StatusCode, String)>::from(e));
            },
        };

        if let Err(e) = Self::service(&state).ensure_upload_allowed(storage_id, &user).await {
            cleanup_tmp(&tmp_path);
            return Err(<(StatusCode, String)>::from(e));
        }

        // Browser File API exposes lastModified; birthtime is usually unavailable, so
        // fall back to mtime for "created" when the client omitted it.
        let source_created_at = source_created_at.or(source_mtime);

        let chunk_size_bytes =
            i64::try_from(state.config.default_chunk_size_bytes()).unwrap_or(i64::MAX);
        let in_file = InFile::new(path, file_size, storage_id)
            .with_chunk_size(chunk_size_bytes)
            .with_source_times(source_created_at, source_mtime)
            .with_content_hash(content_hash);
        let (progress_tx, progress_rx) = mpsc::channel(64);
        let db = state.db.clone();
        let client_tx = state.tx.clone();
        let base_url = state.config.telegram_api_base_url.clone();
        let rate_limit = state.config.telegram_rate_limit;
        let work_dir = state.config.work_dir.clone();
        let user = user.clone();
        let tmp_for_task = tmp_path.clone();

        let upload_task = tokio::spawn(async move {
            let result = FilesService::new(&db, client_tx, &base_url, rate_limit, &work_dir)
                .upload_anyway_from_path_with_progress(
                    in_file,
                    tmp_for_task.clone(),
                    file_size,
                    &user,
                    Some(progress_tx),
                    client_thumb,
                )
                .await;
            if result.is_err() {
                let _ = tokio::fs::remove_file(&tmp_for_task).await;
            }
            result
        });

        Ok(Self::ndjson_upload_progress_response(progress_rx, upload_task))
    }

    async fn create_folder(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath(storage_id): RoutePath<Uuid>,
        Json(params): Json<UploadParams>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        let in_schema = InFolderSchema::new(storage_id, params.path, params.folder_name);

        Self::service(&state).create_folder(in_schema, &user).await?;
        Ok(StatusCode::CREATED)
    }

    /// Stream NDJSON upload progress (`phase=spooled|telegram|waiting|heartbeat|done|error`).
    ///
    /// `spooled` is emitted after the multipart is on disk and the DB row exists, before
    /// Telegram starts — clients may overlap the next file's client→Sarca upload.
    ///
    /// Heartbeats are emitted while Telegram is quiet (Storage Manager queue wait or long
    /// flood sleeps) so reverse proxies / browsers do not idle-timeout the response.
    ///
    /// If the client disconnects (`AbortSignal` / tab close), the upload task is aborted so
    /// unlimited flood-wait retries do not continue in the background.
    fn ndjson_upload_progress_response(
        mut progress_rx: mpsc::Receiver<UploadProgressEvent>,
        upload_task: tokio::task::JoinHandle<SarcaResult<()>>,
    ) -> Response {
        struct AbortOnDrop(Option<tokio::task::AbortHandle>);
        impl Drop for AbortOnDrop {
            fn drop(&mut self) {
                if let Some(h) = self.0.take() {
                    h.abort();
                }
            }
        }
        impl AbortOnDrop {
            fn disarm(&mut self) {
                self.0.take();
            }
        }

        /// How often to push a keepalive NDJSON line when progress is silent.
        const HEARTBEAT_SECS: u64 = 15;

        let abort_guard = AbortOnDrop(Some(upload_task.abort_handle()));
        let stream = async_stream::stream! {
            let mut abort_guard = abort_guard;
            let mut upload_task = upload_task;
            let mut progress_open = true;
            let mut heartbeat =
                tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_SECS));
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Consume the immediate first tick so we don't heartbeat before any real event.
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    ev = progress_rx.recv(), if progress_open => {
                        match ev {
                            Some(ev) => {
                                // Real progress resets the idle heartbeat timer.
                                heartbeat.reset();
                                if let Ok(mut line) = serde_json::to_string(&ev) {
                                    line.push('\n');
                                    yield Ok::<Bytes, std::io::Error>(Bytes::from(line));
                                }
                            }
                            None => progress_open = false,
                        }
                    }
                    _ = heartbeat.tick() => {
                        // Keep the HTTP response alive during SM queue / flood silence.
                        if let Ok(mut line) = serde_json::to_string(&UploadProgressEvent::heartbeat())
                        {
                            line.push('\n');
                            yield Ok::<Bytes, std::io::Error>(Bytes::from(line));
                        }
                    }
                    joined = &mut upload_task => {
                        // Task finished on its own — don't abort on stream drop.
                        abort_guard.disarm();
                        while let Ok(ev) = progress_rx.try_recv() {
                            if let Ok(mut line) = serde_json::to_string(&ev) {
                                line.push('\n');
                                yield Ok(Bytes::from(line));
                            }
                        }
                        match joined {
                            Ok(Ok(())) => {
                                yield Ok(Bytes::from("{\"phase\":\"done\"}\n"));
                            }
                            Ok(Err(e)) => {
                                let (_status, msg) = <(StatusCode, String)>::from(e);
                                let line = serde_json::json!({
                                    "phase": "error",
                                    "message": msg,
                                })
                                .to_string()
                                    + "\n";
                                yield Ok(Bytes::from(line));
                            }
                            Err(e) => {
                                // JoinError from abort is expected on client cancel; prefer a
                                // clear canceled message over a panic string.
                                let message = if e.is_cancelled() {
                                    "Upload canceled".to_owned()
                                } else {
                                    e.to_string()
                                };
                                let line = serde_json::json!({
                                    "phase": "error",
                                    "message": message,
                                })
                                .to_string()
                                    + "\n";
                                yield Ok(Bytes::from(line));
                            }
                        }
                        break;
                    }
                }
            }
        };

        let mut response = (
            StatusCode::CREATED,
            [(header::CONTENT_TYPE, "application/x-ndjson")],
            Body::from_stream(stream),
        )
            .into_response();
        // Discourage reverse proxies / CDNs from buffering NDJSON progress lines.
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-cache, no-transform"),
        );
        response.headers_mut().insert(
            header::HeaderName::from_static("x-accel-buffering"),
            header::HeaderValue::from_static("no"),
        );
        response
    }

    /// Basename only — browsers may put `dir/file.ext` into multipart filename
    /// when uploading a folder (`webkitdirectory`).
    fn file_basename(filename: &str) -> String {
        filename.trim().rsplit(['/', '\\']).next().unwrap_or("").trim().to_string()
    }

    /// Normalize a parent folder path: Unicode/spaces OK, reject `..`, drop empty/`.` segments.
    fn normalize_parent(parent: &str) -> SarcaResult<String> {
        let mut parts = Vec::new();
        for part in parent.split(['/', '\\']) {
            let part = part.trim();
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                return Err(SarcaError::InvalidPath);
            }
            parts.push(part);
        }
        Ok(parts.join("/"))
    }

    /// Parse a browser `File.lastModified`-style epoch (ms or seconds) into UTC.
    fn parse_epoch_millis(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let n: i64 = trimmed.parse().ok()?;
        let ms = if n.abs() < 1_000_000_000_000 { n.saturating_mul(1000) } else { n };
        chrono::DateTime::from_timestamp_millis(ms)
    }

    /// Join a parent folder path with a file name into a logical FS file path.
    /// Avoids `Path::join("")` → trailing `/` (folder marker).
    fn construct_path(parent: &str, filename: &str) -> SarcaResult<String> {
        let parent = Self::normalize_parent(parent)?;
        let filename = Self::file_basename(filename);
        if filename.is_empty() || filename == "." || filename == ".." {
            return Err(SarcaError::InvalidPath);
        }
        let path = if parent.is_empty() { filename } else { format!("{parent}/{filename}") };
        if path.ends_with('/') {
            return Err(SarcaError::InvalidPath);
        }
        Ok(path)
    }

    async fn download(
        state: Arc<AppState>,
        user: AuthUser,
        storage_id: Uuid,
        path: &str,
        query: &SearchQuery,
        headers: &HeaderMap,
    ) -> Result<Response, (StatusCode, String)> {
        check_access(&AccessRepository::new(&state.db), user.id, storage_id, &AccessType::R)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        if path.starts_with('/') || path.contains("//") {
            return Err((StatusCode::BAD_REQUEST, SarcaError::InvalidPath.to_string()));
        }

        if path.ends_with('/') {
            return Self::download_folder(state, storage_id, path).await;
        }

        let files_repo = FilesRepository::new(&state.db);
        match files_repo.get_file_by_path(path, storage_id).await {
            Ok(file) => {
                return Self::download_file(state, storage_id, path, file, query, headers).await;
            },
            Err(SarcaError::DoesNotExist(_)) => {
                // UI folder paths omit the trailing slash; try as folder.
                let folder_path = format!("{path}/");
                match Self::download_folder(state, storage_id, &folder_path).await {
                    Err((StatusCode::NOT_FOUND, _)) => {
                        Err((
                            StatusCode::NOT_FOUND,
                            SarcaError::DoesNotExist("file".to_owned()).to_string(),
                        ))
                    },
                    other => other,
                }
            },
            Err(e) => Err(<(StatusCode, String)>::from(e)),
        }
    }

    pub(crate) async fn download_file(
        state: Arc<AppState>,
        storage_id: Uuid,
        path: &str,
        file: crate::models::files::File,
        query: &SearchQuery,
        headers: &HeaderMap,
    ) -> Result<Response, (StatusCode, String)> {
        let files_repo = FilesRepository::new(&state.db);

        let mut chunks =
            files_repo.list_chunks_of_file(file.id).await.map_err(<(StatusCode, String)>::from)?;
        chunks.sort_by_key(|c| c.position);

        let file_size = file.size.max(0) as u64;
        let chunk_size = file
            .chunk_size_bytes
            .filter(|&n| n > 0)
            .map_or_else(|| state.config.default_chunk_size_bytes(), |n| n as u64);

        let filename =
            Path::new(&path).file_name().and_then(|name| name.to_str()).unwrap_or("unnamed.bin");
        let content_type = mime_guess::from_path(filename).first_or_octet_stream().to_string();

        let want_inline = matches!(query.inline.as_deref(), Some("1" | "true" | "yes"))
            || is_inline_previewable(&content_type);
        let disposition =
            content_disposition_value(if want_inline { "inline" } else { "attachment" }, filename);
        let active = is_active_content(&content_type);

        let range =
            parse_bytes_range(headers.get(header::RANGE).and_then(|v| v.to_str().ok()), file_size);

        let (start, end, status) = match range {
            Ok(None) => (0u64, file_size.saturating_sub(1), StatusCode::OK),
            Ok(Some((s, e))) => (s, e, StatusCode::PARTIAL_CONTENT),
            Err(()) => {
                return Err((
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    format!("Requested range not satisfiable; file size is {file_size}"),
                ));
            },
        };

        // Empty file
        if file_size == 0 {
            let body = Body::from_stream(futures::stream::empty::<Result<Bytes, io::Error>>());
            let mut response = body.into_response();
            *response.status_mut() = StatusCode::OK;
            let headers_mut = response.headers_mut();
            headers_mut.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
            headers_mut.insert(header::CONTENT_DISPOSITION, disposition);
            headers_mut.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
            headers_mut.insert(header::CONTENT_LENGTH, "0".parse().unwrap());
            apply_user_content_headers(headers_mut, active);
            return Ok(response);
        }

        let end = end.min(file_size.saturating_sub(1));
        if start > end {
            return Err((
                StatusCode::RANGE_NOT_SATISFIABLE,
                format!("Requested range not satisfiable; file size is {file_size}"),
            ));
        }

        let content_length = end - start + 1;
        let first_chunk_idx = (start / chunk_size) as usize;
        let last_chunk_idx = (end / chunk_size) as usize;

        let base_url = state.config.telegram_api_base_url.clone();
        let rate = state.config.telegram_rate_limit;
        let db = state.db.clone();
        let cache = ChunkCache::new(&state.config.work_dir);
        let is_video = crate::models::files::is_video(path, Some(&content_type));
        let media_semaphore = state.media_semaphore.clone();
        // Prefetch is a background warm, not the read the caller is waiting
        // on — it shares the thumb semaphore so it can never eat a permit the
        // foreground Range read below needs.
        let prefetch_semaphore = state.thumb_semaphore.clone();

        let channels =
            ordered_active_channels(&db, storage_id).await.map_err(<(StatusCode, String)>::from)?;
        if channels.is_empty() {
            return Err(<(StatusCode, String)>::from(SarcaError::NoActiveChannel));
        }
        let candidates = resolve_chunk_candidates(&db, file.id, &channels)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        fn primary_candidate(candidates: &ChunkCandidates, position: i16) -> Option<String> {
            candidates.get(&position).and_then(|v| v.first()).map(|(f, _)| f.clone())
        }

        // Warm the next Telegram chunk while the player consumes the current Range.
        if is_video {
            if let Some(next_chunk) = chunks.get(last_chunk_idx + 1) {
                if let Some(file_id) = primary_candidate(&candidates, next_chunk.position) {
                    prefetch_telegram_chunk(
                        cache.clone(),
                        base_url.clone(),
                        db.clone(),
                        rate,
                        storage_id,
                        file_id,
                        prefetch_semaphore.clone(),
                    );
                }
            }
        }

        let chunks_positions: Vec<i16> = chunks.iter().map(|c| c.position).collect();

        let stream = try_stream! {
            let mut remaining = content_length;
            let mut cursor = start;

            for (idx, chunk) in chunks.into_iter().enumerate() {
                if idx < first_chunk_idx || idx > last_chunk_idx || remaining == 0 {
                    continue;
                }

                let chunk_start = idx as u64 * chunk_size;
                let skip = cursor.saturating_sub(chunk_start);

                if is_video {
                    if let Some(next_chunk) = chunks_positions.get(idx + 1) {
                        if let Some(file_id) = primary_candidate(&candidates, *next_chunk) {
                            prefetch_telegram_chunk(
                                cache.clone(),
                                base_url.clone(),
                                db.clone(),
                                rate,
                                storage_id,
                                file_id,
                                prefetch_semaphore.clone(),
                            );
                        }
                    }
                }

                let chunk_candidates = candidates.get(&chunk.position).cloned().unwrap_or_default();
                let permit =
                    media_semaphore.acquire().await.expect("media semaphore never closed");
                let cached = ensure_chunk_cached(&cache, &base_url, &db, rate, storage_id, &chunk_candidates)
                    .await
                    .map_err(|e| io::Error::other(e.to_string()))?;
                drop(permit);

                let mut file = tokio::fs::File::open(&cached)
                    .await
                    .map_err(|e| io::Error::other(e.to_string()))?;
                if skip > 0 {
                    file.seek(std::io::SeekFrom::Start(skip))
                        .await
                        .map_err(|e| io::Error::other(e.to_string()))?;
                }

                let mut buf = vec![0u8; 64 * 1024];
                while remaining > 0 {
                    let n = file
                        .read(&mut buf)
                        .await
                        .map_err(|e| io::Error::other(e.to_string()))?;
                    if n == 0 {
                        break;
                    }
                    let take = (n as u64).min(remaining) as usize;
                    remaining -= take as u64;
                    cursor += take as u64;
                    yield Bytes::copy_from_slice(&buf[..take]);
                }
            }
        };

        let stream: Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>> = Box::pin(stream);
        let body = Body::from_stream(stream);

        let mut response = (body).into_response();
        *response.status_mut() = status;

        let headers_mut = response.headers_mut();
        headers_mut.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
        headers_mut.insert(header::CONTENT_DISPOSITION, disposition);
        headers_mut.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
        headers_mut.insert(header::CONTENT_LENGTH, content_length.to_string().parse().unwrap());
        if status == StatusCode::PARTIAL_CONTENT {
            headers_mut.insert(
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{file_size}").parse().unwrap(),
            );
        }
        apply_user_content_headers(headers_mut, active);

        Ok(response)
    }

    // 10 GiB

    pub(crate) async fn download_folder(
        state: Arc<AppState>,
        storage_id: Uuid,
        path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        let prefix = {
            let trimmed = path.trim_end_matches('/');
            if trimmed.is_empty() || trimmed.contains("//") || trimmed.starts_with('/') {
                return Err((StatusCode::BAD_REQUEST, SarcaError::InvalidPath.to_string()));
            }
            format!("{trimmed}/")
        };

        let files_repo = FilesRepository::new(&state.db);

        let total_size = files_repo
            .sum_uploaded_size_under(storage_id, &prefix)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        let files = files_repo
            .list_uploaded_files_under(storage_id, &prefix)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        let folder_marker_exists = files_repo.get_file_by_path(&prefix, storage_id).await.is_ok();

        if !folder_marker_exists && files.is_empty() {
            return Err(<(StatusCode, String)>::from(SarcaError::DoesNotExist(
                "folder".to_owned(),
            )));
        }

        if total_size > Self::MAX_FOLDER_ZIP_BYTES {
            return Err(<(StatusCode, String)>::from(SarcaError::FolderTooLargeForZip));
        }

        let folder_name = Path::new(prefix.trim_end_matches('/'))
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("folder")
            .to_owned();

        let zip_dir = Path::new(&state.config.work_dir).join("zips");
        tokio::fs::create_dir_all(&zip_dir).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Can't create zip directory under WORK_DIR: {e}"),
            )
        })?;

        let zip_path = zip_dir.join(format!("{}.zip", Uuid::new_v4()));
        let zip_path_str = zip_path.to_string_lossy().to_string();

        {
            let zip_file = std::fs::File::create(&zip_path).map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Can't create zip file: {e}"))
            })?;
            let mut zip = zip::ZipWriter::new(zip_file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            let base_url = state.config.telegram_api_base_url.clone();
            let rate = state.config.telegram_rate_limit;
            let db = state.db.clone();

            let channels = ordered_active_channels(&db, storage_id).await.map_err(|e| {
                let _ = std::fs::remove_file(&zip_path);
                <(StatusCode, String)>::from(e)
            })?;
            if channels.is_empty() {
                let _ = std::fs::remove_file(&zip_path);
                return Err(<(StatusCode, String)>::from(SarcaError::NoActiveChannel));
            }

            for file in files {
                let entry_name = file.path.strip_prefix(&prefix).unwrap_or(&file.path).to_owned();
                if entry_name.is_empty() {
                    continue;
                }
                // Never emit a zip-slip entry: a stored path containing `..`
                // would escape the extraction directory on the client.
                if entry_name.starts_with('/')
                    || entry_name.split('/').any(|seg| seg == ".." || seg == ".")
                {
                    continue;
                }

                zip.start_file(&entry_name, options).map_err(|e| {
                    let _ = std::fs::remove_file(&zip_path);
                    (StatusCode::INTERNAL_SERVER_ERROR, format!("Can't write zip entry: {e}"))
                })?;

                let mut chunks = files_repo.list_chunks_of_file(file.id).await.map_err(|e| {
                    let _ = std::fs::remove_file(&zip_path);
                    <(StatusCode, String)>::from(e)
                })?;
                chunks.sort_by_key(|c| c.position);

                let candidates =
                    resolve_chunk_candidates(&db, file.id, &channels).await.map_err(|e| {
                        let _ = std::fs::remove_file(&zip_path);
                        <(StatusCode, String)>::from(e)
                    })?;

                for chunk in chunks {
                    let chunk_candidates =
                        candidates.get(&chunk.position).cloned().unwrap_or_default();
                    let _permit = state
                        .media_semaphore
                        .acquire()
                        .await
                        .expect("media semaphore never closed");
                    let mut stream = download_chunk_stream_with_failover(
                        &base_url,
                        &db,
                        rate,
                        storage_id,
                        &chunk_candidates,
                    )
                    .await
                    .map_err(|e| {
                        let _ = std::fs::remove_file(&zip_path);
                        <(StatusCode, String)>::from(e)
                    })?;

                    while let Some(item) = stream.next().await {
                        let bytes = item.map_err(|e| {
                            let _ = std::fs::remove_file(&zip_path);
                            <(StatusCode, String)>::from(e)
                        })?;
                        use std::io::Write;
                        zip.write_all(&bytes).map_err(|e| {
                            let _ = std::fs::remove_file(&zip_path);
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("Can't write zip data: {e}"),
                            )
                        })?;
                    }
                }
            }

            zip.finish().map_err(|e| {
                let _ = std::fs::remove_file(&zip_path);
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Can't finalize zip: {e}"))
            })?;
        }

        let zip_len = tokio::fs::metadata(&zip_path).await.map_or(0, |m| m.len());

        let stream = try_stream! {
            let mut file = tokio::fs::File::open(&zip_path_str)
                .await
                .map_err(|e| io::Error::other(e.to_string()))?;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = tokio::io::AsyncReadExt::read(&mut file, &mut buf)
                    .await
                    .map_err(|e| io::Error::other(e.to_string()))?;
                if n == 0 {
                    break;
                }
                yield Bytes::copy_from_slice(&buf[..n]);
            }
            let _ = tokio::fs::remove_file(&zip_path_str).await;
        };

        let stream: Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>> = Box::pin(stream);
        let body = Body::from_stream(stream);

        let disposition = content_disposition_value("attachment", &format!("{folder_name}.zip"));
        let mut response = body.into_response();
        *response.status_mut() = StatusCode::OK;
        let headers_mut = response.headers_mut();
        headers_mut.insert(header::CONTENT_TYPE, "application/zip".parse().unwrap());
        headers_mut.insert(header::CONTENT_DISPOSITION, disposition);
        headers_mut.insert(header::CONTENT_LENGTH, zip_len.to_string().parse().unwrap());

        Ok(response)
    }

    async fn thumb(
        state: Arc<AppState>,
        user: AuthUser,
        storage_id: Uuid,
        path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        check_access(&AccessRepository::new(&state.db), user.id, storage_id, &AccessType::R)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        Self::thumb_for_path(state, storage_id, path).await
    }

    /// Thumbnail streaming without access check (caller must authorize).
    pub(crate) async fn thumb_for_path(
        state: Arc<AppState>,
        storage_id: Uuid,
        path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        if path.starts_with('/') || path.contains("//") {
            return Err((StatusCode::BAD_REQUEST, SarcaError::InvalidPath.to_string()));
        }

        let files_repo = FilesRepository::new(&state.db);
        let file = files_repo
            .get_file_by_path(path, storage_id)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        let Some(thumb_id) = file.thumb_telegram_file_id.as_deref() else {
            return Err((StatusCode::NOT_FOUND, "Thumbnail not found".to_owned()));
        };

        let thumb_cache = MediaCache::thumbs(&state.config.work_dir);
        let cache_key = thumb_cache.key(storage_id, path);

        if let Some(bytes) = cached_jpeg(&thumb_cache, &cache_key).await {
            return Ok(inline_jpeg_response(bytes, "thumb.jpg"));
        }

        // A grid renders the same tile from several components and the
        // neighbours race; see the comment at `preview_for_path` above.
        let _flight = SingleFlight::global().acquire(&format!("thumb:{cache_key}")).await;
        if let Some(bytes) = cached_jpeg(&thumb_cache, &cache_key).await {
            return Ok(inline_jpeg_response(bytes, "thumb.jpg"));
        }

        let scheduler = StorageWorkersScheduler::new(&state.db, state.config.telegram_rate_limit);
        // Thumbs get their own reserved-smaller semaphore so a scrolled folder
        // can never fill every `media_semaphore` permit and delay the one
        // preview/inline/download the user is actually waiting on.
        let permit = state.thumb_semaphore.acquire().await.expect("thumb semaphore never closed");
        let deadline = Instant::now() + Self::MEDIA_READ_TOKEN_BUDGET;
        let result = TelegramBotApi::new(&state.config.telegram_api_base_url, scheduler)
            .download_before(thumb_id, storage_id, deadline)
            .await;
        drop(permit);
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(SarcaError::StorageBusy) => return Ok(storage_busy_response()),
            Err(e) => return Err(<(StatusCode, String)>::from(e)),
        };

        // Thumbnails stored while `THUMB_MAX_EDGE` was 1920 are preview-sized
        // documents. Fit them before they are cached, so the whole pre-existing
        // library shrinks on first read instead of re-filling (and re-evicting)
        // the thumb cache with a few hundred oversized tiles.
        let bytes = thumbnails::fit_thumb(bytes).await;

        if is_jpeg(&bytes) {
            if let Err(e) = thumb_cache.put(&cache_key, &bytes).await {
                tracing::warn!("thumb cache write skipped: {e}");
            }
        }

        Ok(inline_jpeg_response(bytes, "thumb.jpg"))
    }

    async fn preview(
        state: Arc<AppState>,
        user: AuthUser,
        storage_id: Uuid,
        path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        check_access(&AccessRepository::new(&state.db), user.id, storage_id, &AccessType::R)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        Self::preview_for_path(state, storage_id, path).await
    }

    /// Preview JPEG streaming without access check (caller must authorize).
    pub(crate) async fn preview_for_path(
        state: Arc<AppState>,
        storage_id: Uuid,
        path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        if path.starts_with('/') || path.contains("//") {
            return Err((StatusCode::BAD_REQUEST, SarcaError::InvalidPath.to_string()));
        }

        let files_repo = FilesRepository::new(&state.db);
        let file = files_repo
            .get_file_by_path(path, storage_id)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        if !preview_allowed(path, file.preview_telegram_file_id.is_some()) {
            return Err((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Preview is only available for image files".to_owned(),
            ));
        }

        let preview_cache = MediaCache::previews(&state.config.work_dir);
        let cache_key = preview_cache.key(storage_id, path);

        if let Some(bytes) = cached_jpeg(&preview_cache, &cache_key).await {
            return Ok(preview_jpeg_response(bytes));
        }

        // The viewer and its neighbor prefetch race for the same photo, and the
        // slow path below costs a full-file download plus a re-encode. One
        // request pays it; the rest wait here and read the result off disk.
        let _flight = SingleFlight::global().acquire(&format!("preview:{cache_key}")).await;
        if let Some(bytes) = cached_jpeg(&preview_cache, &cache_key).await {
            return Ok(preview_jpeg_response(bytes));
        }

        // Preview built at upload time: one small Telegram document instead of
        // re-downloading every chunk of the original and re-encoding it.
        if let Some(preview_id) = file.preview_telegram_file_id.as_deref() {
            let scheduler =
                StorageWorkersScheduler::new(&state.db, state.config.telegram_rate_limit);
            let permit =
                state.media_semaphore.acquire().await.expect("media semaphore never closed");
            let deadline = Instant::now() + Self::MEDIA_READ_TOKEN_BUDGET;
            let result = TelegramBotApi::new(&state.config.telegram_api_base_url, scheduler)
                .download_before(preview_id, storage_id, deadline)
                .await;
            drop(permit);
            match result {
                Ok(bytes) if is_jpeg(&bytes) => {
                    if let Err(e) = preview_cache.put(&cache_key, &bytes).await {
                        tracing::warn!("preview cache write skipped: {e}");
                    }
                    return Ok(preview_jpeg_response(bytes));
                },
                Ok(_) => tracing::warn!("stored preview for {path} is not a JPEG; re-encoding"),
                Err(SarcaError::StorageBusy) => return Ok(storage_busy_response()),
                // Fall through to the slow path: the file predates stored previews, or
                // its preview document is gone from Telegram.
                Err(e) => tracing::warn!("stored preview download failed for {path}: {e}"),
            }
        }

        // Slow-path re-encode stays image-only: there is no on-demand video
        // transcode here, so a video with no (or a lost) stored preview 415s.
        if !thumbnails::is_preview_image(path) {
            return Err((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Preview is only available for image files".to_owned(),
            ));
        }

        let raw = assemble_file_bytes(&state, storage_id, &file).await?;
        let jpeg = thumbnails::generate_preview(raw).await.map_err(|e| {
            (StatusCode::UNSUPPORTED_MEDIA_TYPE, format!("Could not encode preview: {e}"))
        })?;

        if let Err(e) = preview_cache.put(&cache_key, &jpeg).await {
            tracing::warn!("preview cache write skipped: {e}");
        }

        // This file predates stored previews (or its preview document is gone),
        // so every cache eviction re-pays the full download-and-re-encode above.
        // Upload this re-encode as the stored preview so only the first miss
        // ever costs that much again. Detached: the response below must not
        // wait on it.
        let db = state.db.clone();
        let base_url = state.config.telegram_api_base_url.clone();
        let rate = state.config.telegram_rate_limit;
        let work_dir = state.config.work_dir.clone();
        let file_id = file.id;
        let backfill_jpeg = jpeg.clone();
        tokio::spawn(async move {
            StorageManagerService::new(&db, &base_url, rate, &work_dir)
                .backfill_preview(file_id, backfill_jpeg)
                .await;
        });

        Ok(preview_jpeg_response(jpeg))
    }

    /// Need path with trailing slash
    async fn search(
        state: Arc<AppState>,
        user: AuthUser,
        storage_id: Uuid,
        path: &str,
        search_path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        FilesService::new(
            &state.db,
            state.tx.clone(),
            &state.config.telegram_api_base_url,
            state.config.telegram_rate_limit,
            &state.config.work_dir,
        )
        .search(storage_id, path, search_path, &user)
        .await
        .map(|files| Json(files).into_response())
        .map_err(<(StatusCode, String)>::from)
    }

    async fn delete(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath((storage_id, path)): RoutePath<(Uuid, String)>,
    ) -> Result<(), (StatusCode, String)> {
        Self::service(&state)
            .delete(&path, storage_id, &user)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        Ok(())
    }

    async fn rename(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath(storage_id): RoutePath<Uuid>,
        Json(body): Json<RenameSchema>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        let (old_path, new_path) = match (body.old_path, body.new_path, body.path, body.new_name) {
            (Some(old), Some(new), ..) => (old, new),
            (_, _, Some(path), Some(new_name)) => {
                let new = FilesService::rename_with_new_name(&path, &new_name)?;
                (path, new)
            },
            _ => {
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "Provide either {old_path, new_path} or {path, new_name}".to_owned(),
                ));
            },
        };

        Self::service(&state).rename(storage_id, &old_path, &new_path, &user).await?;
        Ok(StatusCode::OK)
    }

    async fn move_to(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath(storage_id): RoutePath<Uuid>,
        Json(body): Json<MoveSchema>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state)
            .move_to(
                storage_id,
                &body.path,
                &body.destination_folder,
                body.on_conflict.as_deref(),
                &user,
            )
            .await?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn copy_to(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath(storage_id): RoutePath<Uuid>,
        Json(body): Json<CopySchema>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        Self::service(&state)
            .copy_to(
                storage_id,
                &body.path,
                &body.destination_folder,
                body.on_conflict.as_deref(),
                &user,
            )
            .await?;
        Ok(StatusCode::NO_CONTENT)
    }
}

/// `telegram_file_id` + `channel_id` candidates for a chunk, ordered by channel priority.
type ChunkCandidates = HashMap<i16, Vec<(String, Uuid)>>;

/// Active channels of a storage, ordered by download priority: current primary first,
/// then the rest by position. Empty if the storage has no active channel.
async fn ordered_active_channels(
    db: &sqlx::SqlitePool,
    storage_id: Uuid,
) -> SarcaResult<Vec<StorageChannel>> {
    let storage = StoragesRepository::new(db).get_by_id(storage_id).await?;
    let mut channels: Vec<StorageChannel> = StorageChannelsRepository::new(db)
        .list_by_storage(storage_id)
        .await?
        .into_iter()
        .filter(super::super::models::storage_channels::StorageChannel::is_active)
        .collect();
    channels.sort_by_key(|c| {
        if c.position == storage.primary_position { (0i16, c.position) } else { (1i16, c.position) }
    });
    Ok(channels)
}

/// For every chunk position of `file_id`, collect the `telegram_file_id` + `channel_id` of
/// each active channel that already has it replicated, ordered by channel priority.
async fn resolve_chunk_candidates(
    db: &sqlx::SqlitePool,
    file_id: Uuid,
    channels: &[StorageChannel],
) -> SarcaResult<ChunkCandidates> {
    let files_repo = FilesRepository::new(db);
    let mut map: ChunkCandidates = HashMap::new();
    for channel in channels {
        let replicas = files_repo.list_chunks_with_replica_for_channel(file_id, channel.id).await?;
        for r in replicas {
            map.entry(r.position).or_default().push((r.telegram_file_id, channel.id));
        }
    }
    Ok(map)
}

/// Fetch (and cache) a chunk's bytes, trying each channel candidate in priority order
/// and marking channels dead when Telegram reports them unreachable.
async fn ensure_chunk_cached(
    cache: &ChunkCache,
    base_url: &str,
    db: &sqlx::SqlitePool,
    rate: u16,
    storage_id: Uuid,
    candidates: &[(String, Uuid)],
) -> SarcaResult<std::path::PathBuf> {
    let mut last_err = SarcaError::DoesNotExist("chunk on any replicated channel".to_owned());
    for (telegram_file_id, channel_id) in candidates {
        let scheduler = StorageWorkersScheduler::new(db, rate);
        let api = TelegramBotApi::new(base_url, scheduler);
        match cache.ensure(telegram_file_id, storage_id, &api).await {
            Ok(path) => return Ok(path),
            Err(e) => {
                tracing::warn!("[DOWNLOAD] chunk fetch failed via channel {channel_id}: {e}");
                if is_chat_dead_error(&e) {
                    let _ = StorageChannelsRepository::new(db).mark_dead(*channel_id).await;
                }
                last_err = e;
            },
        }
    }
    Err(last_err)
}

/// Pull a file's *first* chunk into the on-disk chunk cache.
///
/// Used by the startup/background warmer for videos: the player's opening Range
/// request only ever needs chunk 0, so warming just that one makes "tap the
/// video, it starts" hold without paying for the whole file up front — the rest
/// is prefetched chunk-by-chunk by the streaming path once playback starts.
pub(crate) async fn warm_first_chunk(
    state: &Arc<AppState>,
    storage_id: Uuid,
    path: &str,
) -> SarcaResult<()> {
    let file = FilesRepository::new(&state.db).get_file_by_path(path, storage_id).await?;

    let channels = ordered_active_channels(&state.db, storage_id).await?;
    if channels.is_empty() {
        return Err(SarcaError::NoActiveChannel);
    }

    let candidates = resolve_chunk_candidates(&state.db, file.id, &channels).await?;
    let Some(first_position) = candidates.keys().min().copied() else {
        return Ok(());
    };
    let Some(first) = candidates.get(&first_position) else {
        return Ok(());
    };

    // Background work shares the smaller thumb semaphore, never the media one
    // an interactive read waits on.
    let _permit = state.thumb_semaphore.acquire().await.expect("thumb semaphore never closed");
    ensure_chunk_cached(
        &ChunkCache::new(&state.config.work_dir),
        &state.config.telegram_api_base_url,
        &state.db,
        state.config.telegram_rate_limit,
        storage_id,
        first,
    )
    .await
    .map(|_| ())
}

/// Same as [`ensure_chunk_cached`] but streams straight from Telegram (used for ZIP
/// folder downloads, which don't benefit from the on-disk chunk cache).
async fn download_chunk_stream_with_failover(
    base_url: &str,
    db: &sqlx::SqlitePool,
    rate: u16,
    storage_id: Uuid,
    candidates: &[(String, Uuid)],
) -> SarcaResult<Pin<Box<dyn Stream<Item = Result<tokio_util::bytes::Bytes, SarcaError>> + Send>>> {
    let mut last_err = SarcaError::DoesNotExist("chunk on any replicated channel".to_owned());
    for (telegram_file_id, channel_id) in candidates {
        let scheduler = StorageWorkersScheduler::new(db, rate);
        let api = TelegramBotApi::new(base_url, scheduler);
        match api.download_stream(telegram_file_id, storage_id).await {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                tracing::warn!("[DOWNLOAD] zip chunk fetch failed via channel {channel_id}: {e}");
                if is_chat_dead_error(&e) {
                    let _ = StorageChannelsRepository::new(db).mark_dead(*channel_id).await;
                }
                last_err = e;
            },
        }
    }
    Err(last_err)
}

/// Read all chunk bytes for a file (same Telegram path as download).
async fn assemble_file_bytes(
    state: &AppState,
    storage_id: Uuid,
    file: &crate::models::files::File,
) -> Result<Vec<u8>, (StatusCode, String)> {
    let files_repo = FilesRepository::new(&state.db);
    let mut chunks =
        files_repo.list_chunks_of_file(file.id).await.map_err(<(StatusCode, String)>::from)?;
    chunks.sort_by_key(|c| c.position);

    let file_size = file.size.max(0) as u64;
    if file_size == 0 {
        return Ok(Vec::new());
    }

    let chunk_size = file
        .chunk_size_bytes
        .filter(|&n| n > 0)
        .map_or_else(|| state.config.default_chunk_size_bytes(), |n| n as u64);

    let base_url = state.config.telegram_api_base_url.clone();
    let rate = state.config.telegram_rate_limit;
    let db = state.db.clone();
    let cache = ChunkCache::new(&state.config.work_dir);

    let channels =
        ordered_active_channels(&db, storage_id).await.map_err(<(StatusCode, String)>::from)?;
    if channels.is_empty() {
        return Err(<(StatusCode, String)>::from(SarcaError::NoActiveChannel));
    }
    let candidates = resolve_chunk_candidates(&db, file.id, &channels)
        .await
        .map_err(<(StatusCode, String)>::from)?;

    let mut out = Vec::with_capacity(file_size as usize);
    let _permit = state.media_semaphore.acquire().await.expect("media semaphore never closed");

    for (idx, chunk) in chunks.into_iter().enumerate() {
        let chunk_candidates = candidates.get(&chunk.position).cloned().unwrap_or_default();
        let cached =
            ensure_chunk_cached(&cache, &base_url, &db, rate, storage_id, &chunk_candidates)
                .await
                .map_err(<(StatusCode, String)>::from)?;

        let chunk_start = idx as u64 * chunk_size;
        let remaining = file_size.saturating_sub(chunk_start);
        let to_read = chunk_size.min(remaining) as usize;

        let mut file_handle = tokio::fs::File::open(&cached)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut buf = vec![0u8; to_read];
        let mut read = 0usize;
        while read < to_read {
            let n = file_handle
                .read(&mut buf[read..])
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            if n == 0 {
                break;
            }
            read += n;
        }
        out.extend_from_slice(&buf[..read]);
    }

    Ok(out)
}

/// Ceiling for a client-supplied thumbnail.
///
/// A real photo's tile at `THUMB_MAX_EDGE` is tens of KB, but incompressible
/// detail can push one past 300KB, so the ceiling is set clear of the *worst*
/// case rather than the typical one — see
/// `thumbnails::tests::a_client_built_tile_fits_under_the_upload_ceiling`. Even
/// so it is an order of magnitude under the original being uploaded alongside
/// it, so the field still cannot smuggle a second copy of the photo through.
///
/// A ceiling below what the client actually produces is a silent trap: it was
/// left at its 128px-era value when the edge went to 1920, which dropped the
/// tile for every photo detailed enough to clear it, with nothing logged.
pub(crate) const MAX_CLIENT_THUMB_BYTES: usize = 512 * 1024;

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 3 && bytes[0..3] == [0xFF, 0xD8, 0xFF]
}

fn preview_allowed(path: &str, has_stored_preview: bool) -> bool {
    has_stored_preview || thumbnails::is_preview_image(path)
}

/// Cached JPEG for `key`, dropping the entry if it turns out not to be one
/// (a truncated write, or bytes from before the format was pinned).
async fn cached_jpeg(cache: &MediaCache, key: &str) -> Option<Vec<u8>> {
    let bytes = cache.get(key).await?;
    if is_jpeg(&bytes) {
        return Some(bytes);
    }
    cache.remove(key).await;
    None
}

fn preview_jpeg_response(bytes: Vec<u8>) -> Response {
    inline_jpeg_response(bytes, "preview.jpg")
}

/// 503 with `Retry-After` so `thumbQueue.js` backs off instead of hammering a
/// storage that is already at its Telegram rate limit.
fn storage_busy_response() -> Response {
    let mut response =
        (StatusCode::SERVICE_UNAVAILABLE, SarcaError::StorageBusy.to_string()).into_response();
    response.headers_mut().insert(header::RETRY_AFTER, "5".parse().unwrap());
    response
}

/// `Vec<u8>` responds as `application/octet-stream`, so the JPEG headers must
/// *replace* the defaults — appending them would emit two Content-Type values.
fn inline_jpeg_response(bytes: Vec<u8>, filename: &str) -> Response {
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, "image/jpeg".parse().unwrap());
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("inline; filename=\"{filename}\"").parse().unwrap(),
    );
    headers.insert(header::CACHE_CONTROL, "private, max-age=86400".parse().unwrap());
    response
}

fn prefetch_telegram_chunk(
    cache: ChunkCache,
    base_url: String,
    db: sqlx::SqlitePool,
    rate: u16,
    storage_id: Uuid,
    telegram_file_id: String,
    semaphore: Arc<tokio::sync::Semaphore>,
) {
    tokio::spawn(async move {
        let Ok(_permit) = semaphore.acquire().await else { return };
        let scheduler = StorageWorkersScheduler::new(&db, rate);
        let api = TelegramBotApi::new(&base_url, scheduler);
        if let Err(e) = cache.ensure(&telegram_file_id, storage_id, &api).await {
            tracing::debug!("video chunk prefetch failed: {e}");
        }
    });
}

/// Build a valid `Content-Disposition` header for possibly non-ASCII filenames.
///
/// Plain `filename="…"` must be ASCII (`HeaderValue` rejects Unicode). Use an
/// ASCII fallback plus RFC 5987 `filename*=UTF-8''…` so Cyrillic / spaces work.
fn content_disposition_value(disposition: &str, filename: &str) -> header::HeaderValue {
    let ascii_name: String = filename
        .chars()
        .map(|c| {
            match c {
                ' '..='~' if c != '"' && c != '\\' => c,
                _ => '_',
            }
        })
        .collect();
    let ascii_name =
        if ascii_name.chars().all(|c| c == '_') { "download".to_owned() } else { ascii_name };
    let encoded =
        percent_encoding::utf8_percent_encode(filename, percent_encoding::NON_ALPHANUMERIC);
    let value = format!("{disposition}; filename=\"{ascii_name}\"; filename*=UTF-8''{encoded}");
    value
        .parse()
        .unwrap_or_else(|_| header::HeaderValue::from_static("attachment; filename=\"download\""))
}

#[cfg(test)]
mod active_content_tests {
    use axum::http::{HeaderMap, header};

    use super::{apply_user_content_headers, is_active_content};

    #[test]
    fn script_capable_types_are_treated_as_active() {
        for ct in [
            "text/html",
            "text/html; charset=utf-8",
            "IMAGE/SVG+XML",
            "application/xhtml+xml",
            "application/xml",
        ] {
            assert!(is_active_content(ct), "{ct} must be treated as active content");
        }
    }

    #[test]
    fn media_types_stay_inert() {
        for ct in ["image/png", "video/mp4", "audio/mpeg", "application/pdf", "text/plain"] {
            assert!(!is_active_content(ct), "{ct} must not be treated as active content");
        }
    }

    #[test]
    fn active_content_is_sandboxed_and_never_sniffed() {
        let mut headers = HeaderMap::new();
        apply_user_content_headers(&mut headers, true);
        assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
        let csp = headers.get(header::CONTENT_SECURITY_POLICY).unwrap().to_str().unwrap();
        assert!(csp.starts_with("sandbox"), "sandbox must be first: {csp}");
        assert!(!csp.contains("allow-scripts"), "sandbox must not re-enable scripts: {csp}");

        let mut headers = HeaderMap::new();
        apply_user_content_headers(&mut headers, false);
        assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
        assert!(headers.get(header::CONTENT_SECURITY_POLICY).is_none());
    }
}

#[cfg(test)]
mod content_disposition_tests {
    use super::content_disposition_value;

    #[test]
    fn ascii_filename_parses() {
        let v = content_disposition_value("inline", "transcript.md");
        let s = v.to_str().unwrap();
        assert!(s.contains("filename=\"transcript.md\""));
        assert!(s.contains("filename*=UTF-8''"));
        assert!(s.contains("transcript"));
    }

    #[test]
    fn cyrillic_filename_is_ascii_header() {
        let v = content_disposition_value("inline", "1 часть.mp4");
        assert!(v.to_str().is_ok(), "HeaderValue must be ASCII-safe");
        let s = v.to_str().unwrap();
        assert!(s.contains("filename*=UTF-8''"));
        assert!(s.contains("%D1%87"));
    }
}

/// Whether the mime type is rendered by the browser as an *active* document,
/// i.e. one that can run script in the origin that served it.
///
/// User-uploaded files are served from the application's own origin, so an
/// uploaded `.html` or `.svg` would otherwise execute with full same-origin
/// privileges (reading the caller's tokens from `localStorage`).
fn is_active_content(content_type: &str) -> bool {
    let base = content_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    matches!(
        base.as_str(),
        "text/html"
            | "text/xhtml"
            | "text/xml"
            | "text/xsl"
            | "image/svg+xml"
            | "image/svg"
            | "application/xhtml+xml"
            | "application/xml"
            | "application/mathml+xml"
    )
}

/// Headers applied to every response that streams user-uploaded bytes.
///
/// `nosniff` stops the browser from upgrading an opaque type into an active
/// one. `Content-Security-Policy: sandbox` (no `allow-scripts`) puts active
/// documents into an opaque origin with scripting disabled, which neutralises
/// stored XSS even when the file is opened by direct navigation. It is applied
/// only to active types so that the PDF viewer and media playback keep working.
fn apply_user_content_headers(headers: &mut HeaderMap, active: bool) {
    headers.insert("x-content-type-options", header::HeaderValue::from_static("nosniff"));
    if active {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            header::HeaderValue::from_static(
                "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src data:",
            ),
        );
    }
}

/// Whether the mime type should default to inline preview.
fn is_inline_previewable(content_type: &str) -> bool {
    content_type.starts_with("image/")
        || content_type.starts_with("video/")
        || content_type.starts_with("audio/")
        || content_type == "application/pdf"
        || content_type.starts_with("text/")
}

/// Parse `Range: bytes=start-end`. Returns `Ok(None)` if no range.
/// `Err(())` if the range is invalid / unsatisfiable.
fn parse_bytes_range(header: Option<&str>, file_size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(header) = header else {
        return Ok(None);
    };
    let header = header.trim();
    if file_size == 0 {
        return Err(());
    }
    let Some(spec) = header.strip_prefix("bytes=") else {
        return Err(());
    };
    // Only single range supported
    if spec.contains(',') {
        return Err(());
    }
    let (start_s, end_s) = spec.split_once('-').ok_or(())?;
    if start_s.is_empty() {
        // suffix: bytes=-N
        let n: u64 = end_s.parse().map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        let start = file_size.saturating_sub(n);
        return Ok(Some((start, file_size - 1)));
    }
    let start: u64 = start_s.parse().map_err(|_| ())?;
    if start >= file_size {
        return Err(());
    }
    let end = if end_s.is_empty() {
        file_size - 1
    } else {
        end_s.parse::<u64>().map_err(|_| ())?.min(file_size - 1)
    };
    if end < start {
        return Err(());
    }
    Ok(Some((start, end)))
}

#[cfg(test)]
mod construct_path_tests {
    use super::{FilesRouter, preview_allowed};
    use crate::errors::SarcaError;

    #[test]
    fn root_file() {
        assert_eq!(FilesRouter::construct_path("", "photo.jpg").unwrap(), "photo.jpg");
        assert_eq!(FilesRouter::construct_path("/", "photo.jpg").unwrap(), "photo.jpg");
    }

    #[test]
    fn nested_parent_trims_slash() {
        assert_eq!(FilesRouter::construct_path("docs/", "a.png").unwrap(), "docs/a.png");
        assert_eq!(FilesRouter::construct_path("docs", "a.png").unwrap(), "docs/a.png");
    }

    #[test]
    fn rejects_empty_or_traversal_filename() {
        assert!(matches!(FilesRouter::construct_path("docs", ""), Err(SarcaError::InvalidPath)));
        assert!(matches!(FilesRouter::construct_path("docs", ".."), Err(SarcaError::InvalidPath)));
        assert!(matches!(
            FilesRouter::construct_path("docs/..", "a.png"),
            Err(SarcaError::InvalidPath)
        ));
    }

    #[test]
    fn uses_basename_from_relative_multipart_filename() {
        assert_eq!(
            FilesRouter::construct_path(
                "Пассивный доход до 125 000 ₽. Тариф Премиум (2026)",
                "Пассивный доход до 125 000 ₽. Тариф Премиум (2026)/lesson 1.mp4"
            )
            .unwrap(),
            "Пассивный доход до 125 000 ₽. Тариф Премиум (2026)/lesson 1.mp4"
        );
        assert_eq!(
            FilesRouter::construct_path("docs", r"folder\file.mp4").unwrap(),
            "docs/file.mp4"
        );
    }

    #[test]
    fn trims_segment_edges_keeps_unicode_and_spaces() {
        assert_eq!(
            FilesRouter::construct_path(
                "  Пассивный доход до 125 000 ₽. Тариф Премиум (2026)  ",
                "  video.mp4  "
            )
            .unwrap(),
            "Пассивный доход до 125 000 ₽. Тариф Премиум (2026)/video.mp4"
        );
    }

    #[test]
    fn preview_allowed_images_always() {
        assert!(preview_allowed("a.jpg", false));
        assert!(preview_allowed("a.jpg", true));
    }

    #[test]
    fn preview_allowed_video_only_with_stored_preview() {
        assert!(!preview_allowed("clip.mp4", false));
        assert!(preview_allowed("clip.mp4", true));
    }

    #[test]
    fn preview_allowed_other_types_only_with_stored_preview() {
        assert!(!preview_allowed("doc.pdf", false));
        assert!(preview_allowed("doc.pdf", true));
    }
}
