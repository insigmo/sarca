use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
    time::Duration,
};

use tokio::sync::mpsc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use sarca::{
    common::{channels::ClientMessage, db::pool::get_pool, routing::app_state::AppState},
    conf,
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
    tls::{
        AcmeConfig, CertStore, StubAcmeIssuer, install_crypto_provider, load_or_generate_material,
        new_runtime,
    },
};

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    install_crypto_provider();
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

    match sarca::repositories::files::FilesRepository::new(&db).list_stale_upload_ids().await {
        Ok(ids) if !ids.is_empty() => {
            let n = ids.len();
            match sarca::services::trash::purge_file_ids(
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

    match cleanup_upload_spool(&config.work_dir).await {
        Ok(0) => {},
        Ok(n) => tracing::info!("removed {n} leftover upload spool file(s) under WORK_DIR"),
        Err(e) => tracing::warn!("upload spool cleanup failed: {e}"),
    }

    eprintln!("ensuring superuser…");
    create_superuser(&db, &config).await;
    let config_copy = config.clone();
    let workers = config.workers;

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

    let app_state = AppState::new(db, config.clone(), tx);
    let server = Server::build_server(workers.into(), Arc::new(app_state));

    let plain_http = env::var("SARCA_PLAIN_HTTP").ok().is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let cert_store = CertStore::new(&config.certs_dir);
    cert_store.ensure_dir().await.unwrap_or_else(|e| die(format!("failed to create CERTS_DIR: {e}")));
    let has_certs = cert_store.load_cert().await.ok().flatten().is_some()
        && cert_store.load_key().await.ok().flatten().is_some();
    let tls_mode = !plain_http && (config.tls_hostname.is_some() || has_certs);

    if tls_mode {
        let identity = config.tls_identity().unwrap_or_else(|e| die(format!("invalid TLS_HOSTNAME: {e}")));
        let material = load_or_generate_material(&cert_store, identity.as_ref())
            .await
            .unwrap_or_else(|e| die(format!("failed to load TLS material: {e}")));

        let https_base = config
            .tls_hostname
            .as_ref()
            .map(|h| format!("https://{h}"))
            .unwrap_or_else(|| format!("https://127.0.0.1:{}", config.https_addr.port()));

        let runtime = new_runtime(
            config.https_addr,
            config.acme_http_addr,
            &material,
            https_base.clone(),
        );

        if let Some(id) = identity {
            let _issuer = StubAcmeIssuer::new(AcmeConfig::new(
                config.acme_directory.clone(),
                config.acme_http_addr,
                id,
                runtime.challenges.clone(),
            ));
            tracing::info!(
                "ACME issuer stub ready (directory={}, http={})",
                config.acme_directory,
                config.acme_http_addr
            );
        }

        tracing::info!(
            "TLS mode: HTTPS {} (TCP+H3), ACME http://{}",
            config.https_addr,
            config.acme_http_addr
        );
        server.run_tls(runtime).await;
    } else {
        if plain_http {
            tracing::info!("SARCA_PLAIN_HTTP=1 — plain HTTP on PORT (dev/e2e escape hatch)");
        } else {
            tracing::info!("no TLS_HOSTNAME or certs — plain HTTP on PORT");
        }
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        server.run(&addr).await;
    }
}

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
