use std::{io, path::Path, pin::Pin, sync::Arc};

use axum::{
    body::{Bytes, StreamBody},
    extract::{DefaultBodyLimit, Multipart, Path as RoutePath, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{AppendHeaders, IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use async_stream::try_stream;
use futures::{Stream, StreamExt};
use percent_encoding::percent_decode_str;
use reqwest::header;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{
    common::{
        access::check_access,
        channels::UploadProgressEvent,
        jwt_manager::AuthUser,
        routing::{app_state::AppState, middlewares::auth::logged_in_required},
        telegram_api::bot_api::TelegramBotApi,
    },
    errors::{SarcaError, SarcaResult},
    models::access::AccessType,
    models::files::InFile,
    repositories::{access::AccessRepository, files::FilesRepository},
    schemas::files::{
        InFileSchema, InFolderSchema, MoveSchema, RenameSchema, SearchQuery, UploadParams,
    },
    services::files::FilesService,
    services::storage_workers_scheduler::StorageWorkersScheduler,
};
use tokio::sync::mpsc;

pub struct FilesRouter;

impl FilesRouter {
    pub fn get_router(state: Arc<AppState>) -> Router<Arc<AppState>, axum::body::Body> {
        Router::new()
            .route("/create_folder", post(Self::create_folder))
            .route("/upload", post(Self::upload))
            .route("/upload_to", post(Self::upload_to))
            .route("/rename", post(Self::rename))
            .route("/move", post(Self::move_to))
            .route("/*path", get(Self::dynamic_get).delete(Self::delete))
            .layer(DefaultBodyLimit::disable())
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                logged_in_required,
            ))
            .with_state(state)
    }

    async fn dynamic_get(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath((storage_id, path)): RoutePath<(Uuid, String)>,
        query: Query<SearchQuery>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let (root_path, path) = path.split_once("/").unwrap_or((&path, ""));
        match root_path {
            "tree" => Self::tree(state, user, storage_id, path).await,
            "download" => Self::download(state, user, storage_id, path, &query.0, &headers).await,
            "thumb" => Self::thumb(state, user, storage_id, path).await,
            "search" => {
                if let Some(search_path) = query.0.search_path {
                    Self::search(state, user, storage_id, path, &search_path).await
                } else {
                    Err((
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "search_path query parameter is required".to_owned(),
                    ))
                }
            }
            _ => Err((StatusCode::NOT_FOUND, "Not found".to_owned())),
        }
    }

    async fn tree(
        state: Arc<AppState>,
        user: AuthUser,
        storage_id: Uuid,
        path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        let fs_layer = FilesService::new(&state.db, state.tx.clone())
            .list_dir(storage_id, path, &user)
            .await?;
        Ok(Json(fs_layer).into_response())
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
                format!(
                    "Can't create upload directory under WORK_DIR (check permissions): {e}"
                ),
            )
        })?;

        let tmp_path = upload_dir.join(format!("{}.upload", Uuid::new_v4()));
        let mut tmp_file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Can't create temp file".to_owned()))?;

        let (mut filename_field, mut filename_from_file, mut parent_path, mut file_size) =
            (None::<String>, None::<String>, None::<String>, 0i64);

        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid multipart".to_owned()))?
        {
            let name = field.name().unwrap_or("").to_owned();

            match name.as_str() {
                "file" => {
                    let raw_name = field.file_name().unwrap_or("").to_owned();
                    if !raw_name.trim().is_empty() {
                        filename_from_file = Some(raw_name);
                    }
                    while let Some(chunk) = field
                        .chunk()
                        .await
                        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid file stream".to_owned()))?
                    {
                        file_size += chunk.len() as i64;
                        tmp_file
                            .write_all(&chunk)
                            .await
                            .map_err(|_| {
                                (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "Can't write temp file".to_owned(),
                                )
                            })?;
                    }
                }
                "filename" => {
                    let raw_name = field
                        .text()
                        .await
                        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid filename".to_owned()))?;
                    let decoded = percent_decode_str(&raw_name).decode_utf8_lossy();
                    if !decoded.trim().is_empty() {
                        filename_field = Some(decoded.into_owned());
                    }
                }
                "path" => {
                    let raw_path = field
                        .text()
                        .await
                        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid path".to_owned()))?;
                    let decoded = percent_decode_str(&raw_path).decode_utf8_lossy();
                    parent_path = Some(decoded.into_owned());
                }
                _ => (),
            }
        }

        tmp_file
            .flush()
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Can't flush temp file".to_owned()))?;

        let parent_path =
            parent_path.ok_or((StatusCode::BAD_REQUEST, "path field is required".to_owned()))?;
        let filename = filename_field
            .or(filename_from_file)
            .unwrap_or_else(|| "unnamed".to_owned());
        let path = Self::construct_path(&parent_path, &filename)?;

        FilesService::new(&state.db, state.tx.clone())
            .ensure_upload_allowed(storage_id, &user)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        let in_file = InFile::new(path, file_size, storage_id);
        let (progress_tx, progress_rx) = mpsc::channel(64);
        let db = state.db.clone();
        let client_tx = state.tx.clone();
        let user = user.clone();
        let tmp_for_task = tmp_path.clone();

        let upload_task = tokio::spawn(async move {
            let result = FilesService::new(&db, client_tx)
                .upload_anyway_from_path_with_progress(
                    in_file,
                    tmp_for_task.clone(),
                    file_size,
                    &user,
                    Some(progress_tx),
                )
                .await;
            if result.is_err() {
                let _ = tokio::fs::remove_file(&tmp_for_task).await;
            }
            result
        });

        Ok(Self::ndjson_upload_progress_response(progress_rx, upload_task))
    }

    async fn upload_to(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath(storage_id): RoutePath<Uuid>,
        mut multipart: Multipart,
    ) -> Result<Response, (StatusCode, String)> {
        let upload_dir = Path::new(&state.config.work_dir).join("uploads");
        tokio::fs::create_dir_all(&upload_dir).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "Can't create upload directory under WORK_DIR (check permissions): {e}"
                ),
            )
        })?;

        let tmp_path = upload_dir.join(format!("{}.upload", Uuid::new_v4()));
        let mut tmp_file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Can't create temp file".to_owned()))?;

        // `path` is the destination folder (may be empty for root).
        let (mut filename_field, mut filename_from_file, mut parent_path, mut file_size) =
            (None::<String>, None::<String>, None::<String>, 0i64);

        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid multipart".to_owned()))?
        {
            let name = field.name().unwrap_or("").to_owned();
            match name.as_str() {
                "path" => {
                    let raw_path = field
                        .text()
                        .await
                        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid path".to_owned()))?;
                    let decoded = percent_decode_str(&raw_path).decode_utf8_lossy();
                    parent_path = Some(decoded.into_owned());
                }
                "filename" => {
                    let raw_name = field
                        .text()
                        .await
                        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid filename".to_owned()))?;
                    let decoded = percent_decode_str(&raw_name).decode_utf8_lossy();
                    if !decoded.trim().is_empty() {
                        filename_field = Some(decoded.into_owned());
                    }
                }
                "file" => {
                    let raw_name = field.file_name().unwrap_or("").to_owned();
                    if !raw_name.trim().is_empty() {
                        filename_from_file = Some(raw_name);
                    }
                    while let Some(chunk) = field
                        .chunk()
                        .await
                        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid file stream".to_owned()))?
                    {
                        file_size += chunk.len() as i64;
                        tmp_file
                            .write_all(&chunk)
                            .await
                            .map_err(|_| {
                                (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "Can't write temp file".to_owned(),
                                )
                            })?;
                    }
                }
                _ => (),
            }
        }

        tmp_file
            .flush()
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Can't flush temp file".to_owned()))?;

        let parent_path =
            parent_path.ok_or((StatusCode::BAD_REQUEST, "path field is required".to_owned()))?;
        let filename = filename_field
            .or(filename_from_file)
            .unwrap_or_else(|| "unnamed".to_owned());
        let path = Self::construct_path(&parent_path, &filename)?;

        FilesService::new(&state.db, state.tx.clone())
            .ensure_upload_allowed(storage_id, &user)
            .await
            .map_err(<(StatusCode, String)>::from)?;

        let in_schema = InFileSchema::new(storage_id, path, tmp_path.clone(), file_size);
        let (progress_tx, progress_rx) = mpsc::channel(64);
        let db = state.db.clone();
        let client_tx = state.tx.clone();
        let user = user.clone();
        let tmp_for_task = tmp_path.clone();

        let upload_task = tokio::spawn(async move {
            let result = FilesService::new(&db, client_tx)
                .upload_to_with_progress(in_schema, &user, Some(progress_tx))
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

        FilesService::new(&state.db, state.tx.clone())
            .create_folder(in_schema, &user)
            .await?;
        Ok(StatusCode::CREATED)
    }

    /// Stream NDJSON upload progress (`phase=telegram|done|error`) while Telegram upload runs.
    fn ndjson_upload_progress_response(
        mut progress_rx: mpsc::Receiver<UploadProgressEvent>,
        upload_task: tokio::task::JoinHandle<SarcaResult<()>>,
    ) -> Response {
        let stream = async_stream::stream! {
            let mut upload_task = upload_task;
            let mut progress_open = true;
            loop {
                tokio::select! {
                    ev = progress_rx.recv(), if progress_open => {
                        match ev {
                            Some(ev) => {
                                if let Ok(mut line) = serde_json::to_string(&ev) {
                                    line.push('\n');
                                    yield Ok::<Bytes, std::io::Error>(Bytes::from(line));
                                }
                            }
                            None => progress_open = false,
                        }
                    }
                    joined = &mut upload_task => {
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
                                let line = serde_json::json!({
                                    "phase": "error",
                                    "message": e.to_string(),
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

        (
            StatusCode::CREATED,
            [(header::CONTENT_TYPE, "application/x-ndjson")],
            StreamBody::new(stream),
        )
            .into_response()
    }

    /// Basename only — browsers may put `dir/file.ext` into multipart filename
    /// when uploading a folder (`webkitdirectory`).
    fn file_basename(filename: &str) -> String {
        filename
            .trim()
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
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

    /// Join a parent folder path with a file name into a logical FS file path.
    /// Avoids `Path::join("")` → trailing `/` (folder marker).
    fn construct_path(parent: &str, filename: &str) -> SarcaResult<String> {
        let parent = Self::normalize_parent(parent)?;
        let filename = Self::file_basename(filename);
        if filename.is_empty() || filename == "." || filename == ".." {
            return Err(SarcaError::InvalidPath);
        }
        let path = if parent.is_empty() {
            filename
        } else {
            format!("{parent}/{filename}")
        };
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
        check_access(
            &AccessRepository::new(&state.db),
            user.id,
            storage_id,
            &AccessType::R,
        )
        .await
        .map_err(|e| <(StatusCode, String)>::from(e))?;

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
            }
            Err(SarcaError::DoesNotExist(_)) => {
                // UI folder paths omit the trailing slash; try as folder.
                let folder_path = format!("{path}/");
                return Self::download_folder(state, storage_id, &folder_path).await;
            }
            Err(e) => return Err(<(StatusCode, String)>::from(e)),
        }
    }

    async fn download_file(
        state: Arc<AppState>,
        storage_id: Uuid,
        path: &str,
        file: crate::models::files::File,
        query: &SearchQuery,
        headers: &HeaderMap,
    ) -> Result<Response, (StatusCode, String)> {
        let files_repo = FilesRepository::new(&state.db);

        let mut chunks = files_repo
            .list_chunks_of_file(file.id)
            .await
            .map_err(|e| <(StatusCode, String)>::from(e))?;
        chunks.sort_by_key(|c| c.position);

        let file_size = file.size.max(0) as u64;
        let chunk_size =
            (state.config.telegram_chunk_size_mb as u64).saturating_mul(1024 * 1024).max(1);

        let filename = Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unnamed.bin");
        let content_type = mime_guess::from_path(filename)
            .first_or_octet_stream()
            .to_string();

        let want_inline = matches!(
            query.inline.as_deref(),
            Some("1") | Some("true") | Some("yes")
        ) || is_inline_previewable(&content_type);
        let disposition = if want_inline {
            format!("inline; filename=\"{filename}\"")
        } else {
            format!("attachment; filename=\"{filename}\"")
        };

        let range = parse_bytes_range(
            headers
                .get(header::RANGE)
                .and_then(|v| v.to_str().ok()),
            file_size,
        );

        let (start, end, status) = match range {
            Ok(None) => (0u64, file_size.saturating_sub(1), StatusCode::OK),
            Ok(Some((s, e))) => (s, e, StatusCode::PARTIAL_CONTENT),
            Err(()) => {
                return Err((
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    format!("Requested range not satisfiable; file size is {file_size}"),
                ));
            }
        };

        // Empty file
        if file_size == 0 {
            let body = StreamBody::new(futures::stream::empty::<Result<Bytes, io::Error>>());
            let mut response = body.into_response();
            *response.status_mut() = StatusCode::OK;
            let headers_mut = response.headers_mut();
            headers_mut.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
            headers_mut.insert(header::CONTENT_DISPOSITION, disposition.parse().unwrap());
            headers_mut.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
            headers_mut.insert(header::CONTENT_LENGTH, "0".parse().unwrap());
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

        let stream = try_stream! {
            let mut remaining = content_length;
            let mut cursor = start;

            for (idx, chunk) in chunks.into_iter().enumerate() {
                if idx < first_chunk_idx || idx > last_chunk_idx || remaining == 0 {
                    continue;
                }

                let chunk_start = idx as u64 * chunk_size;
                let mut skip = cursor.saturating_sub(chunk_start);

                let scheduler = StorageWorkersScheduler::new(&db, rate);
                let api = TelegramBotApi::new(&base_url, scheduler);

                let mut s = api
                    .download_stream(&chunk.telegram_file_id, storage_id)
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

                while let Some(item) = s.next().await {
                    if remaining == 0 {
                        break;
                    }

                    let bytes = item
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

                    if skip > 0 {
                        let blen = bytes.len() as u64;
                        if skip >= blen {
                            skip -= blen;
                            continue;
                        }
                        let from = skip as usize;
                        skip = 0;
                        let slice = bytes.slice(from..);
                        let take = (slice.len() as u64).min(remaining) as usize;
                        remaining -= take as u64;
                        cursor += take as u64;
                        yield slice.slice(..take);
                    } else {
                        let take = (bytes.len() as u64).min(remaining) as usize;
                        remaining -= take as u64;
                        cursor += take as u64;
                        yield bytes.slice(..take);
                    }
                }
            }
        };

        let stream: Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>> = Box::pin(stream);
        let body = StreamBody::new(stream);

        let mut response = (body).into_response();
        *response.status_mut() = status;

        let headers_mut = response.headers_mut();
        headers_mut.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
        headers_mut.insert(header::CONTENT_DISPOSITION, disposition.parse().unwrap());
        headers_mut.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
        headers_mut.insert(
            header::CONTENT_LENGTH,
            content_length.to_string().parse().unwrap(),
        );
        if status == StatusCode::PARTIAL_CONTENT {
            headers_mut.insert(
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{file_size}").parse().unwrap(),
            );
        }

        Ok(response)
    }

    /// Max total uncompressed size of files packed into a folder ZIP.
    const MAX_FOLDER_ZIP_BYTES: i64 = 10 * 1024 * 1024 * 1024; // 10 GiB

    async fn download_folder(
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
            .map_err(|e| <(StatusCode, String)>::from(e))?;

        let files = files_repo
            .list_uploaded_files_under(storage_id, &prefix)
            .await
            .map_err(|e| <(StatusCode, String)>::from(e))?;

        let folder_marker_exists = files_repo
            .get_file_by_path(&prefix, storage_id)
            .await
            .is_ok();

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
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Can't create zip file: {e}"),
                )
            })?;
            let mut zip = zip::ZipWriter::new(zip_file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            let base_url = state.config.telegram_api_base_url.clone();
            let rate = state.config.telegram_rate_limit;
            let db = state.db.clone();

            for file in files {
                let entry_name = file
                    .path
                    .strip_prefix(&prefix)
                    .unwrap_or(&file.path)
                    .to_owned();
                if entry_name.is_empty() {
                    continue;
                }

                zip.start_file(&entry_name, options).map_err(|e| {
                    let _ = std::fs::remove_file(&zip_path);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Can't write zip entry: {e}"),
                    )
                })?;

                let mut chunks = files_repo
                    .list_chunks_of_file(file.id)
                    .await
                    .map_err(|e| {
                        let _ = std::fs::remove_file(&zip_path);
                        <(StatusCode, String)>::from(e)
                    })?;
                chunks.sort_by_key(|c| c.position);

                for chunk in chunks {
                    let scheduler = StorageWorkersScheduler::new(&db, rate);
                    let api = TelegramBotApi::new(&base_url, scheduler);
                    let mut stream = api
                        .download_stream(&chunk.telegram_file_id, storage_id)
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
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Can't finalize zip: {e}"),
                )
            })?;
        }

        let zip_len = tokio::fs::metadata(&zip_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let stream = try_stream! {
            let mut file = tokio::fs::File::open(&zip_path_str)
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = tokio::io::AsyncReadExt::read(&mut file, &mut buf)
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
                if n == 0 {
                    break;
                }
                yield Bytes::copy_from_slice(&buf[..n]);
            }
            let _ = tokio::fs::remove_file(&zip_path_str).await;
        };

        let stream: Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>> = Box::pin(stream);
        let body = StreamBody::new(stream);

        let disposition = format!("attachment; filename=\"{folder_name}.zip\"");
        let mut response = body.into_response();
        *response.status_mut() = StatusCode::OK;
        let headers_mut = response.headers_mut();
        headers_mut.insert(header::CONTENT_TYPE, "application/zip".parse().unwrap());
        headers_mut.insert(header::CONTENT_DISPOSITION, disposition.parse().unwrap());
        headers_mut.insert(
            header::CONTENT_LENGTH,
            zip_len.to_string().parse().unwrap(),
        );

        Ok(response)
    }

    async fn thumb(
        state: Arc<AppState>,
        user: AuthUser,
        storage_id: Uuid,
        path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        check_access(
            &AccessRepository::new(&state.db),
            user.id,
            storage_id,
            &AccessType::R,
        )
        .await
        .map_err(|e| <(StatusCode, String)>::from(e))?;

        if path.starts_with('/') || path.contains("//") {
            return Err((
                StatusCode::BAD_REQUEST,
                SarcaError::InvalidPath.to_string(),
            ));
        }

        let files_repo = FilesRepository::new(&state.db);
        let file = files_repo
            .get_file_by_path(path, storage_id)
            .await
            .map_err(|e| <(StatusCode, String)>::from(e))?;

        let Some(thumb_id) = file.thumb_telegram_file_id.as_deref() else {
            return Err((StatusCode::NOT_FOUND, "Thumbnail not found".to_owned()));
        };

        let scheduler = StorageWorkersScheduler::new(&state.db, state.config.telegram_rate_limit);
        let bytes = TelegramBotApi::new(&state.config.telegram_api_base_url, scheduler)
            .download(thumb_id, storage_id)
            .await
            .map_err(|e| <(StatusCode, String)>::from(e))?;

        let headers = AppendHeaders([
            (header::CONTENT_TYPE, "image/jpeg".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                "inline; filename=\"thumb.jpg\"".to_owned(),
            ),
            (header::CACHE_CONTROL, "private, max-age=86400".to_owned()),
        ]);

        Ok((headers, bytes).into_response())
    }

    ///
    /// Need path with trailing slash
    ///
    async fn search(
        state: Arc<AppState>,
        user: AuthUser,
        storage_id: Uuid,
        path: &str,
        search_path: &str,
    ) -> Result<Response, (StatusCode, String)> {
        FilesService::new(&state.db, state.tx.clone())
            .search(storage_id, path, search_path, &user)
            .await
            .map(|files| Json(files).into_response())
            .map_err(|e| <(StatusCode, String)>::from(e))
    }

    async fn delete(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath((storage_id, path)): RoutePath<(Uuid, String)>,
    ) -> Result<(), (StatusCode, String)> {
        FilesService::new(&state.db, state.tx.clone())
            .delete(&path, storage_id, &user)
            .await
            .map_err(|e| <(StatusCode, String)>::from(e))?;

        Ok(())
    }

    async fn rename(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath(storage_id): RoutePath<Uuid>,
        Json(body): Json<RenameSchema>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        let (old_path, new_path) = match (body.old_path, body.new_path, body.path, body.new_name) {
            (Some(old), Some(new), _, _) => (old, new),
            (_, _, Some(path), Some(new_name)) => {
                let new = FilesService::rename_with_new_name(&path, &new_name)?;
                (path, new)
            }
            _ => {
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "Provide either {old_path, new_path} or {path, new_name}".to_owned(),
                ));
            }
        };

        FilesService::new(&state.db, state.tx.clone())
            .rename(storage_id, &old_path, &new_path, &user)
            .await?;
        Ok(StatusCode::OK)
    }

    async fn move_to(
        State(state): State<Arc<AppState>>,
        Extension(user): Extension<AuthUser>,
        RoutePath(storage_id): RoutePath<Uuid>,
        Json(body): Json<MoveSchema>,
    ) -> Result<StatusCode, (StatusCode, String)> {
        FilesService::new(&state.db, state.tx.clone())
            .move_to(storage_id, &body.path, &body.destination_folder, &user)
            .await?;
        Ok(StatusCode::OK)
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
    use super::FilesRouter;
    use crate::errors::SarcaError;

    #[test]
    fn root_file() {
        assert_eq!(
            FilesRouter::construct_path("", "photo.jpg").unwrap(),
            "photo.jpg"
        );
        assert_eq!(
            FilesRouter::construct_path("/", "photo.jpg").unwrap(),
            "photo.jpg"
        );
    }

    #[test]
    fn nested_parent_trims_slash() {
        assert_eq!(
            FilesRouter::construct_path("docs/", "a.png").unwrap(),
            "docs/a.png"
        );
        assert_eq!(
            FilesRouter::construct_path("docs", "a.png").unwrap(),
            "docs/a.png"
        );
    }

    #[test]
    fn rejects_empty_or_traversal_filename() {
        assert!(matches!(
            FilesRouter::construct_path("docs", ""),
            Err(SarcaError::InvalidPath)
        ));
        assert!(matches!(
            FilesRouter::construct_path("docs", ".."),
            Err(SarcaError::InvalidPath)
        ));
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
}

