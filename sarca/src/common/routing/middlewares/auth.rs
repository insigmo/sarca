use std::sync::Arc;

use axum::{
    extract::State,
    headers::{Authorization, HeaderMapExt, authorization::Bearer},
    http::Request,
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
    repositories::users::UsersRepository,
};

/// Middleware that requires to be logged in.
/// Accepts `Authorization: Bearer …` or `?access_token=` (for `<video>` / `<img>` / `<iframe>`).
pub async fn logged_in_required<B>(
    State(state): State<Arc<AppState>>,
    mut req: Request<B>,
    next: Next<B>,
) -> Result<Response, (StatusCode, String)> {
    let auth_user = authenticate_request(&req, &state.config.secret_key)
        .map_err(<(StatusCode, String)>::from)?;

    // A signature-valid token can still name a user that no longer exists (the row was
    // dropped, e.g. by a db reset). Such a session is not merely empty — it silently
    // owns nothing, so listings look empty and writes fail deep in access checks.
    // Reject it here so the client refreshes and gets a token bound to a live row.
    let user = UsersRepository::new(&state.db)
        .get_by_id(auth_user.id)
        .await
        .map_err(|_| <(StatusCode, String)>::from(SarcaError::NotAuthenticated))?;

    req.extensions_mut().insert(AuthUser::new(user.id, user.email));
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
