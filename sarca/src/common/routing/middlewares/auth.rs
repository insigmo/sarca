use std::sync::Arc;

use axum::{
    extract::State,
    headers::{authorization::Bearer, Authorization, HeaderMapExt},
    http::{HeaderMap, HeaderValue, Request},
    middleware::Next,
    response::Response,
};
use percent_encoding::percent_decode_str;
use reqwest::StatusCode;

use crate::{
    common::{
        jwt_manager::{AuthUser, JWTManager},
        routing::app_state::AppState,
    },
    errors::{SarcaError, SarcaResult},
};

/// Middleware that requires to be logged in.
/// Accepts `Authorization: Bearer …` or `?access_token=` (for `<video>` / `<img>` / `<iframe>`).
pub async fn logged_in_required<B>(
    State(state): State<Arc<AppState>>,
    mut req: Request<B>,
    next: Next<B>,
) -> Result<Response, (StatusCode, String)> {
    let auth_user = authenticate_request(&req, &state.config.secret_key)
        .map_err(|e| <(StatusCode, String)>::from(e))?;

    req.extensions_mut().insert(auth_user);
    Ok(next.run(req).await)
}

fn authenticate_request<B>(req: &Request<B>, secret_key: &str) -> SarcaResult<AuthUser> {
    if let Some(auth_header) = req.headers().typed_get::<Authorization<Bearer>>() {
        return JWTManager::validate(auth_header.token(), secret_key);
    }

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

    Err(SarcaError::NotAuthenticated)
}

#[inline]
#[allow(dead_code)]
fn authenticate(headers: &HeaderMap<HeaderValue>, secret_key: &str) -> SarcaResult<AuthUser> {
    let auth_header = headers
        .typed_get::<Authorization<Bearer>>()
        .ok_or(SarcaError::NotAuthenticated)?;

    JWTManager::validate(auth_header.token(), secret_key)
}
