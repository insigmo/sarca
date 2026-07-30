use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
    time::Duration,
};

use tokio::sync::mpsc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    common::{channels::ClientMessage, db::pool::get_pool, routing::app_state::AppState},
    config::Config,
    server::Server,
    services::{
        channel_health::ChannelHealthService,
        replication::ReplicationService,
        storage_purge::StoragePurgeService,
        trash_purge::TrashPurgeService,
    },
    startup::{create_superuser, delete_orphan_storage_workers, init_db},
    storage_manager::StorageManager,
};

mod common;
mod conf;
mod config;
mod errors;
mod models;
mod repositories;
mod routers;
mod schemas;
mod server;
mod services;
mod startup;
mod storage_manager;
mod tls;

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    // Load sarca.conf (or migrate legacy .env) before reading Config.
    conf::load_sarca_conf();

    let config = Config::new().unwrap_or_else(|e| die(format!("failed to load config: {e}")));

    tokio::fs::create_dir_all(&config.work_dir)
        .await
        .unwrap_or_else(|e| die(format!("failed to create WORK_DIR {}: {e}", config.work_dir)));

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sarca=info,tower_http=info,axum::rejection=trace".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let port = config.port;
    eprintln!("starting Sarca (PORT={port} from config)…");
    tracing::info!("starting Sarca on port {port}");

    let db_timeout = Duration::from_secs(10);
    let (tx, rx) = mpsc::channel::<ClientMessage>(config.channel_capacity.into());

    eprintln!("opening SQLite database at {}…", config.sqlite_path);
    let db = get_pool(&config.sqlite_path, config.workers.into(), db_timeout).await.unwrap_or_else(
        |e| {
            die(format!("{e}\nhint: check SQLITE_PATH in sarca.conf and its directory permissions"))
        },
    );
    eprintln!("database ok");

    eprintln!("initializing schema…");
    init_db(&db).await;
    delete_orphan_storage_workers(&db).await;

    match crate::repositories::files::FilesRepository::new(&db).list_stale_upload_ids().await {
        Ok(ids) if !ids.is_empty() => {
            let n = ids.len();
            match crate::services::trash::purge_file_ids(
                &db,
                &config.telegram_api_base_url,
                config.telegram_rate_limit,
                &ids,
            )
            .await
            {
                Ok(()) => tracing::info!("cleaned up {n} stale unfinished uploads"),
                Err(e) => tracing::warn!("stale upload cleanup failed: {e}"),
            }
        },
        Ok(_) => {},
        Err(e) => tracing::warn!("stale upload cleanup failed: {e}"),
    }

    // Leftover *.upload spools cannot belong to a live request after restart.
    match cleanup_upload_spool(&config.work_dir).await {
        Ok(0) => {},
        Ok(n) => tracing::info!("removed {n} leftover upload spool file(s) under WORK_DIR"),
        Err(e) => tracing::warn!("upload spool cleanup failed: {e}"),
    }

    eprintln!("ensuring superuser…");
    create_superuser(&db, &config).await;
    let config_copy = config.clone();
    let workers = config.workers;

    // One SQLite pool is shared by the HTTP router and every background worker;
    // SQLite serializes writers, so extra pools would only add lock contention.
    let manager_db = db.clone();
    tokio::spawn(async move {
        let mut manager = StorageManager::new(rx, manager_db, config_copy);
        tracing::debug!("running manager");
        manager.run().await;
    });

    ReplicationService::spawn_loop(
        db.clone(),
        config.telegram_api_base_url.clone(),
        config.telegram_rate_limit,
        Duration::from_secs(10),
    );

    ChannelHealthService::spawn_loop(
        db.clone(),
        config.telegram_api_base_url.clone(),
        config.telegram_rate_limit,
        Duration::from_mins(30),
    );

    TrashPurgeService::spawn_loop(
        db.clone(),
        config.telegram_api_base_url.clone(),
        config.telegram_rate_limit,
        Duration::from_mins(10),
    );

    StoragePurgeService::spawn_loop(
        db.clone(),
        config.telegram_api_base_url.clone(),
        config.telegram_rate_limit,
        Duration::from_secs(5),
    );

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
    let app_state = AppState::new(db, config, tx);
    let server = Server::build_server(workers.into(), Arc::new(app_state));
    server.run(&addr).await;
}

/// Remove leftover `*.upload` spool files under `WORK_DIR/uploads`.
/// After a process restart no in-flight multipart can own them.
async fn cleanup_upload_spool(work_dir: &str) -> std::io::Result<usize> {
    let dir = Path::new(work_dir).join("uploads");
    let mut rd = match tokio::fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0usize;
    while let Some(ent) = rd.next_entry().await? {
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".upload") {
            match tokio::fs::remove_file(ent.path()).await {
                Ok(()) => removed += 1,
                Err(e) => tracing::warn!("failed to remove {}: {e}", ent.path().display()),
            }
        }
    }
    Ok(removed)
}
