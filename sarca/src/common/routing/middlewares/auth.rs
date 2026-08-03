use std::sync::Arc;

use axum::{
    extract::State,
    http::{Method, Request, StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::Response,
};
use percent_encoding::percent_decode_str;

use crate::{
    common::{
        jwt_manager::{AuthUser, JWTManager},
        routing::app_state::AppState,
    },
    errors::{SarcaError, SarcaResult},
    repositories::users::UsersRepository,
};

/// Middleware that requires to be logged in.
/// Accepts `Authorization: Bearer …` or `?access_token=` (for `<video>` / `<img>` / `<iframe>`).
pub async fn logged_in_required(
    State(state): State<Arc<AppState>>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, (StatusCode, String)> {
    let auth_user = authenticate_request(&req, &state.config.secret_key)
        .map_err(<(StatusCode, String)>::from)?;

    let user = UsersRepository::new(&state.db)
        .get_by_id(auth_user.id)
        .await
        .map_err(|_| <(StatusCode, String)>::from(SarcaError::NotAuthenticated))?;

    // Reject tokens minted before the last password reset / logout.
    if !user.session_is_live(auth_user.issued_at) {
        return Err(<(StatusCode, String)>::from(SarcaError::NotAuthenticated));
    }

    req.extensions_mut().insert(AuthUser::new(user.id, user.email));
    Ok(next.run(req).await)
}

fn authenticate_request(
    req: &Request<axum::body::Body>,
    secret_key: &str,
) -> SarcaResult<AuthUser> {
    if let Some(token) = bearer_token(req) {
        return JWTManager::validate(&token, secret_key);
    }

    // A token in the query string leaks into browser history, proxy access
    // logs and `Referer` headers, so accept it only where a bare element URL
    // is unavoidable: read-only media GETs. Every other route needs the header.
    if is_media_get(req) {
        if let Some(query) = req.uri().query() {
            for pair in query.split('&') {
                let mut parts = pair.splitn(2, '=');
                if parts.next() == Some("access_token") {
                    if let Some(raw) = parts.next() {
                        let token = percent_decode_str(raw).decode_utf8_lossy();
                        if !token.is_empty() {
                            return JWTManager::validate(&token, secret_key);
                        }
                    }
                }
            }
        }
    }

    Err(SarcaError::NotAuthenticated)
}

/// `GET /api/storages/{id}/files/{download,thumb,preview}/…` — the endpoints
/// loaded directly by `<video>` / `<img>` / `<iframe>`, which cannot send an
/// `Authorization` header.
fn is_media_get(req: &Request<axum::body::Body>) -> bool {
    if req.method() != Method::GET {
        return false;
    }
    let path = req.uri().path();
    let Some((_, rest)) = path.split_once("/files/") else {
        return false;
    };
    let action = rest.split('/').next().unwrap_or_default();
    matches!(action, "download" | "thumb" | "preview")
}

fn bearer_token(req: &Request<axum::body::Body>) -> Option<String> {
    req.headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned)
}

#[cfg(test)]
mod query_token_scope_tests {
    use super::*;

    fn req(method: Method, uri: &str) -> Request<axum::body::Body> {
        Request::builder().method(method).uri(uri).body(axum::body::Body::empty()).unwrap()
    }

    #[test]
    fn media_gets_accept_a_query_token() {
        let base = "/api/storages/11111111-1111-1111-1111-111111111111/files";
        for action in ["download", "thumb", "preview"] {
            assert!(is_media_get(&req(Method::GET, &format!("{base}/{action}/a.mp4"))));
        }
    }

    #[test]
    fn other_routes_and_methods_do_not() {
        let base = "/api/storages/11111111-1111-1111-1111-111111111111/files";
        assert!(!is_media_get(&req(Method::GET, &format!("{base}/tree/"))));
        assert!(!is_media_get(&req(Method::GET, &format!("{base}/info/a.mp4"))));
        assert!(!is_media_get(&req(Method::POST, &format!("{base}/upload"))));
        assert!(!is_media_get(&req(Method::POST, &format!("{base}/download/a.mp4"))));
        assert!(!is_media_get(&req(Method::GET, "/api/users")));
        assert!(!is_media_get(&req(Method::GET, "/api/setup/storages")));
        // A path that merely *mentions* an action must not qualify.
        assert!(!is_media_get(&req(Method::GET, "/api/users/download")));
    }
}
