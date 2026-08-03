use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    Router,
    http::{
        HeaderValue,
        Method,
        StatusCode,
        header::{
            ACCEPT,
            AUTHORIZATION,
            CACHE_CONTROL,
            CONTENT_TYPE,
            HeaderName,
            IF_MODIFIED_SINCE,
            IF_NONE_MATCH,
            RANGE,
        },
    },
};
use tower::{ServiceBuilder, limit::ConcurrencyLimitLayer};
use tower_http::{
    cors,
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    trace::{DefaultOnRequest, TraceLayer},
};
use tracing::Level;

use crate::{
    common::routing::app_state::AppState,
    conf,
    routers::{
        auth::AuthRouter,
        public_shares::PublicSharesRouter,
        settings::SettingsRouter,
        setup::SetupRouter,
        storage_workers::StorageWorkersRouter,
        storages::StoragesRouter,
        users::UsersRouter,
    },
    tls::{TlsRuntime, serve_dual_tls},
};

pub struct Server {
    router: Router,
    ui_dir: PathBuf,
}

impl Server {
    pub fn build_server(workers: usize, app_state: Arc<AppState>) -> Self {
        let ui_dir = resolve_ui_dir();
        let index = ui_dir.join("index.html");
        let assets = ui_dir.join("assets");

        tracing::info!("serving UI from {}", ui_dir.display());

        // Hashed Vite assets are safe to cache forever; index.html must revalidate
        // or browsers keep a stale script src → 404 → white screen after rebuild.
        let serve_assets = ServiceBuilder::new()
            .layer(SetResponseHeaderLayer::overriding(
                CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ))
            .service(ServeDir::new(assets));
        let serve_ui = ServiceBuilder::new()
            .layer(SetResponseHeaderLayer::overriding(
                CACHE_CONTROL,
                HeaderValue::from_static("no-cache"),
            ))
            .service(ServeFile::new(index));

        let router = Router::new()
            .nest("/api", Self::build_api_router(workers, app_state))
            .nest_service("/assets", serve_assets)
            .fallback_service(serve_ui);
        let router = with_security_headers(router);

        Self {
            router,
            ui_dir,
        }
    }

    #[inline]
    fn build_api_router(workers: usize, app_state: Arc<AppState>) -> Router {
        let app_cors = cors_layer();

        Router::new()
            .nest("/users", UsersRouter::get_router(app_state.clone()))
            .nest("/auth", AuthRouter::get_router(app_state.clone()))
            .nest("/storages", StoragesRouter::get_router(app_state.clone()))
            .nest(
                "/storage_workers",
                StorageWorkersRouter::get_router(app_state.clone()),
            )
            .nest("/settings", SettingsRouter::get_router(app_state.clone()))
            .nest("/setup", SetupRouter::get_router(app_state.clone()))
            .nest(
                "/public/shares",
                PublicSharesRouter::get_router(app_state),
            )
            // Keep unknown /api/* from falling through to the SPA HTML fallback.
            .fallback(|| async { (StatusCode::NOT_FOUND, "Not Found") })
            .layer(ConcurrencyLimitLayer::new(workers))
            .layer(app_cors)
            .layer(Self::request_trace_layer())
    }

    /// Per-API-request span. Span itself + request-start line stay debug (only
    /// visible with `DEBUG_LOG=1` / `RUST_LOG`), but the response line's level
    /// tracks the actual status: 5xx → error, 4xx → warn, else debug — so real
    /// failures surface without needing verbose logging on all the time.
    #[allow(clippy::type_complexity)]
    fn request_trace_layer() -> TraceLayer<
        tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>,
        impl Fn(&axum::http::Request<axum::body::Body>) -> tracing::Span + Clone,
        DefaultOnRequest,
        impl Fn(&axum::http::Response<axum::body::Body>, std::time::Duration, &tracing::Span) + Clone,
    > {
        TraceLayer::new_for_http()
            .make_span_with(|request: &axum::http::Request<axum::body::Body>| {
                tracing::debug_span!(
                    "http",
                    method = %request.method(),
                    path = %request.uri().path(),
                )
            })
            .on_request(DefaultOnRequest::new().level(Level::DEBUG))
            .on_response(
                |response: &axum::http::Response<axum::body::Body>,
                 latency: std::time::Duration,
                 _span: &tracing::Span| {
                    let status = response.status();
                    let latency_ms = latency.as_millis();
                    if status.is_server_error() {
                        tracing::error!(%status, latency_ms, "request failed");
                    } else if status.is_client_error() {
                        tracing::warn!(%status, latency_ms, "request rejected");
                    } else {
                        tracing::debug!(%status, latency_ms, "request completed");
                    }
                },
            )
    }

    /// Plain HTTP on `PORT` (e2e / dev when `SARCA_PLAIN_HTTP=1` or no TLS config).
    pub async fn run(self, addr: &SocketAddr) {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap_or_else(|e| {
            eprintln!();
            eprintln!("error: cannot bind to {addr}: {e}");
            eprintln!(
                "hint: port {} is probably already in use — stop the other process or set a free \
                 PORT in {}",
                addr.port(),
                conf::CONF_NAME
            );
            std::process::exit(1);
        });

        let public = format!("http://127.0.0.1:{}", addr.port());
        eprintln!();
        eprintln!("========================================");
        eprintln!("  Sarca is running");
        eprintln!("  UI:      {public}");
        eprintln!("  API:     {public}/api");
        eprintln!("  Listen:  http://{addr}");
        eprintln!("  UI dir:  {}", self.ui_dir.display());
        eprintln!("========================================");
        eprintln!();
        tracing::info!("listening on {public} (bound to {addr})");

        axum::serve(listener, self.router).await.unwrap();
    }

    /// HTTP/3 (UDP) + TCP HTTPS on `HTTPS_ADDR`, ACME http-01 + redirect on `ACME_HTTP_ADDR`.
    pub async fn run_tls(
        self,
        runtime: TlsRuntime,
        acme_task: Option<tokio::task::JoinHandle<()>>,
    ) {
        let ui_dir = self.ui_dir.clone();
        let router = with_alt_svc(self.router, runtime.https_addr.port());
        serve_dual_tls(router, ui_dir, runtime, acme_task).await;
    }

    /// Minimal health router for TLS integration tests (no UI directory required).
    pub fn health_router(https_port: u16) -> Router {
        with_alt_svc(
            Router::new().route("/health", axum::routing::get(|| async { "ok" })),
            https_port,
        )
    }
}

/// Origins allowed to read API responses cross-site, on top of same-origin
/// requests (which carry no `Origin` and never need CORS).
///
/// The Tauri client loads the server UI in its `WebView`, so its API calls are
/// same-origin; these cover the bundled shell pages (`index.html`/`sync.html`),
/// which talk to the API from a `tauri://` origin.
const SHELL_ORIGINS: &[&str] =
    &["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"];

/// Extra origins, comma-separated. `*` restores the old allow-anything
/// behaviour and is only honoured because some deployments front the API with
/// a separate web app; it is never the default.
const CORS_ORIGINS_VAR: &str = "SARCA_CORS_ORIGINS";

/// CORS policy for `/api`.
///
/// This used to be `allow_origin(Any)` with `allow_headers(Any)`, which let any
/// website a user visited read every API response their browser could reach —
/// including, on a fresh instance, `/api/setup`. Requests are now only readable
/// cross-site from origins the operator names.
fn cors_layer() -> cors::CorsLayer {
    let layer = cors::CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::HEAD,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            ACCEPT,
            AUTHORIZATION,
            CONTENT_TYPE,
            RANGE,
            IF_MODIFIED_SINCE,
            IF_NONE_MATCH,
        ])
        .max_age(std::time::Duration::from_mins(10));

    let configured = std::env::var(CORS_ORIGINS_VAR).unwrap_or_default();
    let configured: Vec<&str> =
        configured.split(',').map(str::trim).filter(|o| !o.is_empty()).collect();

    if configured.contains(&"*") {
        tracing::warn!(
            "{CORS_ORIGINS_VAR} contains '*': every website can read API responses. \
             List explicit origins instead."
        );
        return layer.allow_origin(cors::Any);
    }

    let mut origins: Vec<HeaderValue> = Vec::new();
    for origin in SHELL_ORIGINS.iter().copied().chain(configured) {
        if let Ok(value) = HeaderValue::from_str(origin) {
            origins.push(value);
        } else {
            tracing::warn!(origin, "ignoring unparsable {CORS_ORIGINS_VAR} entry");
        }
    }
    layer.allow_origin(origins)
}

/// Response headers that hold for everything this server serves.
///
/// The SPA shipped without any of these: no CSP, so a single injection turned
/// into script execution; no `X-Frame-Options`, so the UI could be framed and
/// clickjacked; no `nosniff`, so an uploaded file served back could be sniffed
/// into HTML and run on the app's own origin.
fn with_security_headers(router: Router) -> Router {
    // `style-src` needs 'unsafe-inline': SUID injects component styles as inline
    // <style> at runtime. Scripts stay 'self' only, which is what stops XSS.
    // Fonts come from Google Fonts (see ui/index.html).
    const CSP: &str = "default-src 'self'; \
         script-src 'self'; \
         style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
         font-src 'self' data: https://fonts.gstatic.com; \
         img-src 'self' data: blob:; \
         media-src 'self' data: blob:; \
         connect-src 'self'; \
         worker-src 'self' blob:; \
         frame-src 'none'; \
         object-src 'none'; \
         base-uri 'self'; \
         form-action 'self'; \
         frame-ancestors 'none'";

    let headers: [(HeaderName, &'static str); 5] = [
        (HeaderName::from_static("content-security-policy"), CSP),
        (HeaderName::from_static("x-content-type-options"), "nosniff"),
        (HeaderName::from_static("x-frame-options"), "DENY"),
        (HeaderName::from_static("referrer-policy"), "strict-origin-when-cross-origin"),
        (
            HeaderName::from_static("permissions-policy"),
            "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), \
             microphone=(), payment=(), usb=()",
        ),
    ];

    headers.into_iter().fold(router, |router, (name, value)| {
        router.layer(SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value)))
    })
}

pub fn with_alt_svc(router: Router, https_port: u16) -> Router {
    let value =
        HeaderValue::from_str(&format!("h3=\":{https_port}\"; ma=86400")).expect("Alt-Svc header");
    router.layer(SetResponseHeaderLayer::overriding(HeaderName::from_static("alt-svc"), value))
}

/// Locate the built UI (`index.html` + `assets/`).
///
/// Search order matches installer layout, then cwd, then cargo/dev layouts.
pub fn resolve_ui_dir() -> PathBuf {
    let candidates = ui_dir_candidates();

    if let Some(dir) = find_ui_dir_among(&candidates) {
        return dir;
    }

    eprintln!();
    eprintln!("error: UI not found (looked for ui/index.html next to the binary and in cwd)");
    eprintln!("searched:");
    for candidate in &candidates {
        eprintln!("  - {}", candidate.display());
    }
    eprintln!("hint: reinstall Sarca, or run from a directory that contains ui/");
    std::process::exit(1);
}

pub fn ui_dir_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("ui"));
            // cargo run: target/{debug,release}/sarca → ../../ui/dist
            candidates.push(exe_dir.join("../../ui/dist"));
            candidates.push(exe_dir.join("../ui"));
        }
    }

    candidates.push(PathBuf::from("ui"));
    candidates.push(PathBuf::from("ui/dist"));
    candidates
}

pub fn find_ui_dir_among(candidates: &[PathBuf]) -> Option<PathBuf> {
    for candidate in candidates {
        if candidate.join("index.html").is_file() {
            return Some(candidate.canonicalize().unwrap_or_else(|_| candidate.clone()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn find_ui_dir_picks_first_with_index() {
        let root = std::env::temp_dir().join(format!("sarca-ui-{}", uuid::Uuid::new_v4()));
        let ui = root.join("ui");
        fs::create_dir_all(ui.join("assets")).unwrap();
        fs::write(ui.join("index.html"), "<html>ok</html>").unwrap();

        let missing = root.join("missing");
        let found = find_ui_dir_among(&[missing, ui.clone()]).unwrap();
        assert_eq!(found, ui.canonicalize().unwrap());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn find_ui_dir_none_when_missing() {
        let root = std::env::temp_dir().join(format!("sarca-ui-miss-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        assert!(find_ui_dir_among(&[root.join("ui")]).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    async fn response_headers(
        router: Router,
        request: axum::http::Request<axum::body::Body>,
    ) -> axum::http::HeaderMap {
        use tower::ServiceExt;
        router.oneshot(request).await.expect("router responds").headers().clone()
    }

    fn cors_router() -> Router {
        Router::new().route("/ping", axum::routing::get(|| async { "pong" })).layer(cors_layer())
    }

    fn get_with_origin(origin: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri("/ping")
            .header("Origin", origin)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    // Regression: the API answered `Access-Control-Allow-Origin: *`, so any page
    // the user visited could read every response their browser could reach.
    #[tokio::test]
    async fn cors_refuses_unknown_origins() {
        let headers =
            response_headers(cors_router(), get_with_origin("https://evil.example.com")).await;
        assert!(
            headers.get("access-control-allow-origin").is_none(),
            "an unlisted origin must not be echoed back: {headers:?}"
        );
    }

    #[tokio::test]
    async fn cors_allows_the_native_shell_origin() {
        let headers = response_headers(cors_router(), get_with_origin("tauri://localhost")).await;
        assert_eq!(
            headers.get("access-control-allow-origin").and_then(|v| v.to_str().ok()),
            Some("tauri://localhost")
        );
        assert!(headers.get("vary").is_some(), "a per-origin reply must vary on Origin");
    }

    #[tokio::test]
    async fn cors_never_allows_credentials() {
        let headers = response_headers(cors_router(), get_with_origin("tauri://localhost")).await;
        assert!(
            headers.get("access-control-allow-credentials").is_none(),
            "cookies must not ride along cross-site"
        );
    }

    #[tokio::test]
    async fn security_headers_are_applied_to_every_response() {
        let router = with_security_headers(
            Router::new().route("/ping", axum::routing::get(|| async { "pong" })),
        );
        let request =
            axum::http::Request::builder().uri("/ping").body(axum::body::Body::empty()).unwrap();
        let headers = response_headers(router, request).await;

        let csp = headers
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok())
            .expect("CSP header");
        assert!(csp.contains("script-src 'self'"));
        assert!(!csp.contains("unsafe-eval"), "eval must stay blocked: {csp}");
        assert!(
            !csp.contains("script-src 'self' 'unsafe-inline'"),
            "inline script must stay blocked: {csp}"
        );
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("object-src 'none'"));

        assert_eq!(
            headers.get("x-content-type-options").and_then(|v| v.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(headers.get("x-frame-options").and_then(|v| v.to_str().ok()), Some("DENY"));
        assert!(headers.get("referrer-policy").is_some());
        assert!(headers.get("permissions-policy").is_some());
    }
}
