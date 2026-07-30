use std::sync::Arc;

use axum::{
    extract::State,
    http::{Request, header::AUTHORIZATION},
    middleware::Next,
    response::Response,
};
use percent_encoding::percent_decode_str;
use axum::http::StatusCode;

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

    req.extensions_mut().insert(AuthUser::new(user.id, user.email));
    Ok(next.run(req).await)
}

fn authenticate_request(req: &Request<axum::body::Body>, secret_key: &str) -> SarcaResult<AuthUser> {
    if let Some(token) = bearer_token(req) {
        return JWTManager::validate(&token, secret_key);
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

fn bearer_token(req: &Request<axum::body::Body>) -> Option<String> {
    req.headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned)
}
