use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
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
        trash_purge::TrashPurgeService,
    },
    startup::{
        bootstrap_storage_from_env,
        create_db,
        create_superuser,
        delete_orphan_storage_workers,
        init_db,
    },
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

    eprintln!("connecting to Postgres…");
    create_db(&config.db_uri_without_dbname, &config.db_name, config.workers.into(), db_timeout)
        .await
        .unwrap_or_else(|e| {
            die(format!("{e}\nhint: check DATABASE_* in sarca.conf and that Postgres is running"))
        });

    let db =
        get_pool(&config.db_uri, config.workers.into(), db_timeout).await.unwrap_or_else(|e| {
            die(format!("{e}\nhint: check DATABASE_* in sarca.conf and that Postgres is running"))
        });
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

    eprintln!("ensuring superuser…");
    create_superuser(&db, &config).await;
    bootstrap_storage_from_env(&db, &config).await;

    let config_copy = config.clone();
    let workers = config.workers;
    tokio::spawn(async move {
        match get_pool(&config_copy.db_uri, workers.into(), db_timeout).await {
            Ok(db) => {
                let mut manager = StorageManager::new(rx, db, config_copy);
                tracing::debug!("running manager");
                manager.run().await;
            },
            Err(e) => tracing::error!("storage manager db pool failed: {e}"),
        }
    });

    match get_pool(&config.db_uri, workers.into(), db_timeout).await {
        Ok(db) => {
            ReplicationService::spawn_loop(
                db,
                config.telegram_api_base_url.clone(),
                config.telegram_rate_limit,
                Duration::from_secs(10),
            );
        },
        Err(e) => tracing::error!("replication worker db pool failed: {e}"),
    }

    match get_pool(&config.db_uri, workers.into(), db_timeout).await {
        Ok(db) => {
            ChannelHealthService::spawn_loop(
                db,
                config.telegram_api_base_url.clone(),
                config.telegram_rate_limit,
                Duration::from_mins(30),
            );
        },
        Err(e) => tracing::error!("channel health scheduler db pool failed: {e}"),
    }

    match get_pool(&config.db_uri, workers.into(), db_timeout).await {
        Ok(db) => {
            TrashPurgeService::spawn_loop(
                db,
                config.telegram_api_base_url.clone(),
                config.telegram_rate_limit,
                Duration::from_mins(10),
            );
        },
        Err(e) => tracing::error!("trash purge scheduler db pool failed: {e}"),
    }

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
    let app_state = AppState::new(db, config, tx);
    let server = Server::build_server(workers.into(), Arc::new(app_state));
    server.run(&addr).await;
}
