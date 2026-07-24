use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use percent_encoding::percent_decode_str;

use crate::{
    common::{jwt_manager::JWTManager, routing::app_state::AppState},
    errors::SarcaError,
    models::share_links::ShareLink,
    repositories::files::FilesRepository,
    routers::files::FilesRouter,
    schemas::{
        files::SearchQuery,
        shares::{NeedPasswordSchema, PublicTreeQuery, UnlockShareSchema},
    },
    services::shares::PublicSharesService,
};

pub struct PublicSharesRouter;

impl PublicSharesRouter {
    pub fn get_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/:token", get(Self::metadata))
            .route("/:token/unlock", post(Self::unlock))
            .route("/:token/tree", get(Self::tree))
            .route("/:token/download_zip", get(Self::download_zip))
            .route("/:token/download", get(Self::download_root))
            .route("/:token/download/*relpath", get(Self::download))
            .route("/:token/thumb", get(Self::thumb_root))
            .route("/:token/thumb/*relpath", get(Self::thumb))
            .route("/:token/inline", get(Self::inline_root))
            .route("/:token/inline/*relpath", get(Self::inline_file))
            .with_state(state)
    }

    fn service(state: &AppState) -> PublicSharesService<'_> {
        PublicSharesService::new(&state.db)
    }

    async fn gate(
        state: &AppState,
        token: &str,
        headers: &HeaderMap,
    ) -> Result<ShareLink, Response> {
        let svc = Self::service(state);
        let link = svc.load_available(token).await.map_err(err_response)?;

        if link.has_password() {
            let cookie_name = unlock_cookie_name(token);
            let unlocked = headers
                .get(header::COOKIE)
                .and_then(|v| v.to_str().ok())
                .and_then(|jar| find_cookie(jar, &cookie_name))
                .is_some_and(|val| {
                    JWTManager::validate_share_unlock(&val, token, &state.config.secret_key).is_ok()
                });

            if !unlocked {
                return Err(
                    (StatusCode::UNAUTHORIZED, Json(NeedPasswordSchema::yes())).into_response()
                );
            }
        }

        Ok(link)
    }

    async fn metadata(
        State(state): State<Arc<AppState>>,
        Path(token): Path<String>,
        headers: HeaderMap,
    ) -> Result<Response, Response> {
        let link = Self::gate(&state, &token, &headers).await?;
        let meta = Self::service(&state).metadata(&link).await.map_err(err_response)?;
        Ok(Json(meta).into_response())
    }

    async fn unlock(
        State(state): State<Arc<AppState>>,
        Path(token): Path<String>,
        Json(body): Json<UnlockShareSchema>,
    ) -> Result<Response, Response> {
        let svc = Self::service(&state);
        let link = svc.load_available(&token).await.map_err(err_response)?;

        if !link.has_password() {
            // No password required; treat as success without cookie.
            return Ok(StatusCode::NO_CONTENT.into_response());
        }

        PublicSharesService::verify_password(&link, &body.password).map_err(|e| {
            match e {
                SarcaError::NotAuthenticated => {
                    (StatusCode::UNAUTHORIZED, Json(NeedPasswordSchema::yes())).into_response()
                },
                other => err_response(other),
            }
        })?;

        let max_age = PublicSharesService::unlock_max_age_secs(&link);
        if max_age == 0 {
            return Err(err_response(SarcaError::DoesNotExist("share link".to_owned())));
        }

        let jwt = JWTManager::generate_share_unlock(
            &token,
            Duration::from_secs(max_age),
            &state.config.secret_key,
        );

        let cookie = format!(
            "{}={}; Path=/api/public/shares/{}; HttpOnly; SameSite=Lax; Max-Age={}",
            unlock_cookie_name(&token),
            jwt,
            token,
            max_age
        );

        let mut response = StatusCode::NO_CONTENT.into_response();
        if let Ok(val) = HeaderValue::from_str(&cookie) {
            response.headers_mut().insert(header::SET_COOKIE, val);
        }
        Ok(response)
    }

    async fn tree(
        State(state): State<Arc<AppState>>,
        Path(token): Path<String>,
        Query(query): Query<PublicTreeQuery>,
        headers: HeaderMap,
    ) -> Result<Response, Response> {
        let link = Self::gate(&state, &token, &headers).await?;
        let relative = query.path.as_deref().unwrap_or("");
        let elements = Self::service(&state).tree(&link, relative).await.map_err(err_response)?;
        Ok(Json(elements).into_response())
    }

    async fn download_zip(
        State(state): State<Arc<AppState>>,
        Path(token): Path<String>,
        headers: HeaderMap,
    ) -> Result<Response, Response> {
        let link = Self::gate(&state, &token, &headers).await?;
        let folder = PublicSharesService::resolve_folder_zip_path(&link).map_err(err_response)?;
        // Confirm still live.
        let _ = Self::service(&state).metadata(&link).await.map_err(err_response)?;
        FilesRouter::download_folder(state, link.storage_id, &folder)
            .await
            .map_err(|(s, m)| (s, m).into_response())
    }

    async fn download_root(
        State(state): State<Arc<AppState>>,
        Path(token): Path<String>,
        headers: HeaderMap,
    ) -> Result<Response, Response> {
        Self::download_inner(state, token, String::new(), &headers, false).await
    }

    async fn download(
        State(state): State<Arc<AppState>>,
        Path((token, relpath)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> Result<Response, Response> {
        let relpath = percent_decode_str(&relpath).decode_utf8_lossy().to_string();
        Self::download_inner(state, token, relpath, &headers, false).await
    }

    async fn inline_root(
        State(state): State<Arc<AppState>>,
        Path(token): Path<String>,
        headers: HeaderMap,
    ) -> Result<Response, Response> {
        Self::download_inner(state, token, String::new(), &headers, true).await
    }

    async fn inline_file(
        State(state): State<Arc<AppState>>,
        Path((token, relpath)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> Result<Response, Response> {
        let relpath = percent_decode_str(&relpath).decode_utf8_lossy().to_string();
        Self::download_inner(state, token, relpath, &headers, true).await
    }

    async fn download_inner(
        state: Arc<AppState>,
        token: String,
        relpath: String,
        headers: &HeaderMap,
        force_inline: bool,
    ) -> Result<Response, Response> {
        let link = Self::gate(&state, &token, headers).await?;
        let abs = PublicSharesService::resolve_file_path(&link, &relpath).map_err(err_response)?;

        let files_repo = FilesRepository::new(&state.db);
        let file =
            files_repo.get_file_by_path(&abs, link.storage_id).await.map_err(err_response)?;

        let query = SearchQuery {
            search_path: None,
            inline: if force_inline { Some("1".to_owned()) } else { None },
        };

        FilesRouter::download_file(state, link.storage_id, &abs, file, &query, headers)
            .await
            .map_err(|(s, m)| (s, m).into_response())
    }

    async fn thumb_root(
        State(state): State<Arc<AppState>>,
        Path(token): Path<String>,
        headers: HeaderMap,
    ) -> Result<Response, Response> {
        Self::thumb_inner(state, token, String::new(), &headers).await
    }

    async fn thumb(
        State(state): State<Arc<AppState>>,
        Path((token, relpath)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> Result<Response, Response> {
        let relpath = percent_decode_str(&relpath).decode_utf8_lossy().to_string();
        Self::thumb_inner(state, token, relpath, &headers).await
    }

    async fn thumb_inner(
        state: Arc<AppState>,
        token: String,
        relpath: String,
        headers: &HeaderMap,
    ) -> Result<Response, Response> {
        let link = Self::gate(&state, &token, headers).await?;
        let abs = PublicSharesService::resolve_file_path(&link, &relpath).map_err(err_response)?;
        FilesRouter::thumb_for_path(state, link.storage_id, &abs)
            .await
            .map_err(|(s, m)| (s, m).into_response())
    }
}

fn unlock_cookie_name(token: &str) -> String {
    format!("share_unlock_{token}")
}

fn find_cookie(cookie_header: &str, name: &str) -> Option<String> {
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            if k.trim() == name {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn err_response(e: SarcaError) -> Response {
    let (status, msg): (StatusCode, String) = e.into();
    (status, msg).into_response()
}
