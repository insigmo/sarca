use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
};

use axum::Router;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    cors,
    services::{ServeDir, ServeFile},
};

use crate::{
    common::routing::app_state::AppState,
    routers::{
        auth::AuthRouter, storage_workers::StorageWorkersRouter, storages::StoragesRouter,
        users::UsersRouter,
    },
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

        let serve_ui = ServeFile::new(index);
        let serve_assets = ServeDir::new(assets);

        let router = Router::new()
            .nest("/api", Self::build_api_router(workers, app_state))
            .nest_service("/assets", serve_assets)
            .fallback_service(serve_ui);

        Self { router, ui_dir }
    }

    #[inline]
    fn build_api_router(workers: usize, app_state: Arc<AppState>) -> Router {
        let app_cors = cors::CorsLayer::new()
            .allow_methods(cors::Any)
            .allow_headers(cors::Any)
            .allow_origin(cors::Any);

        Router::new()
            .nest("/users", UsersRouter::get_router(app_state.clone()))
            .nest("/auth", AuthRouter::get_router(app_state.clone()))
            .nest("/storages", StoragesRouter::get_router(app_state.clone()))
            .nest(
                "/storage_workers",
                StorageWorkersRouter::get_router(app_state.clone()),
            )
            .layer(ConcurrencyLimitLayer::new(workers.into()))
            .layer(app_cors)
    }

    pub async fn run(self, addr: &SocketAddr) {
        let listener = std::net::TcpListener::bind(addr).unwrap_or_else(|e| {
            eprintln!();
            eprintln!("error: cannot bind to {addr}: {e}");
            eprintln!(
                "hint: port {} is probably already in use — stop the other process \
                 or set a free PORT in .env",
                addr.port()
            );
            std::process::exit(1);
        });
        listener
            .set_nonblocking(true)
            .expect("failed to set nonblocking on listener");

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

        axum::Server::from_tcp(listener)
            .expect("failed to create HTTP server from listener")
            .serve(self.router.into_make_service())
            .await
            .unwrap();
    }
}

/// Locate the built UI (`index.html` + `assets/`).
///
/// Search order matches installer layout, then cwd, then cargo/dev layouts.
fn resolve_ui_dir() -> PathBuf {
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

    for candidate in &candidates {
        let index = candidate.join("index.html");
        if index.is_file() {
            return candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone());
        }
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
