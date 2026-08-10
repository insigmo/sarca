use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
    time::Duration,
};

use sarca::{
    common::{
        channels::ClientMessage,
        db::pool::{BACKGROUND_CONNECTIONS, get_pool},
        routing::app_state::AppState,
    },
    conf,
    config::Config,
    server::Server,
    services::{
        channel_health::ChannelHealthService,
        media_warmer,
        replication::ReplicationService,
        storage_purge::StoragePurgeService,
        trash_purge::TrashPurgeService,
    },
    startup::{
        create_superuser,
        delete_orphan_storage_workers,
        init_db,
        reset_previews_on_format_change,
    },
    storage_manager::StorageManager,
    tls::{
        CertStore,
        ChallengeStore,
        InstantAcmeIssuer,
        acme_enabled,
        detect_public_ip,
        install_crypto_provider,
        load_or_generate_material,
        new_runtime,
        shared_identity,
        spawn_acme_http_listener,
        spawn_public_ip_watch,
        spawn_renewal_task,
    },
};
use tokio::sync::mpsc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    install_crypto_provider();
    conf::load_sarca_conf();

    let mut config = Config::new().unwrap_or_else(|e| die(format!("failed to load config: {e}")));

    tokio::fs::create_dir_all(&config.work_dir)
        .await
        .unwrap_or_else(|e| die(format!("failed to create WORK_DIR {}: {e}", config.work_dir)));

    let default_filter = if config.debug_log {
        "sarca=debug,tower_http=debug,axum::rejection=trace"
    } else {
        "sarca=info,tower_http=info,axum::rejection=trace"
    };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Panics in spawned tasks don't kill the process (panic=unwind default), but they
    // were previously invisible under docker's `restart: unless-stopped` — only a bare
    // stderr line, easy to lose. Log every panic at error level with a backtrace so a
    // crash-looping deploy is diagnosable from `docker logs` alone.
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("{info}\n{backtrace}");
    }));

    let port = config.port;
    eprintln!("starting Sarca (PORT={port} from config)…");
    tracing::info!("starting Sarca on port {port}");
    if config.debug_log {
        tracing::info!("DEBUG_LOG=1 — verbose request/action logging enabled");
    }

    let plain_http = env::var("SARCA_PLAIN_HTTP")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

    // No TLS_HOSTNAME: fall back to the machine's public IP so the server still
    // comes up on HTTPS (TCP + HTTP/3) with an ACME-issuable identity. Only a
    // host with no usable address at all drops to plain HTTP.
    let mut identity_auto_detected = false;
    if !plain_http && config.tls_hostname.is_none() {
        eprintln!("TLS_HOSTNAME unset — detecting public IP…");
        match detect_public_ip().await {
            Some(ip) => {
                tracing::info!("TLS_HOSTNAME unset — using detected address {ip} as TLS identity");
                config.tls_hostname = Some(ip.to_string());
                identity_auto_detected = true;
            },
            None => {
                tracing::warn!(
                    "TLS_HOSTNAME unset and no public IP could be detected — plain HTTP on PORT"
                );
            },
        }
    }
    let config = config;

    let db_timeout = Duration::from_secs(10);
    let (tx, rx) = mpsc::channel::<ClientMessage>(config.channel_capacity.into());

    eprintln!("opening SQLite database at {}…", config.sqlite_path);
    // Background loops need their own slots; sizing the pool to WORKERS alone
    // let them starve the HTTP handlers under load.
    let db_connections = u32::from(config.workers) + BACKGROUND_CONNECTIONS;
    let db = get_pool(&config.sqlite_path, db_connections, db_timeout).await.unwrap_or_else(|e| {
        die(format!("{e}\nhint: check SQLITE_PATH in sarca.conf and its directory permissions"))
    });
    eprintln!("database ok");

    eprintln!("initializing schema…");
    init_db(&db).await;
    delete_orphan_storage_workers(&db).await;
    reset_previews_on_format_change(&db, config.work_dir.as_ref()).await;

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

    // Not supervised: the manager owns the single `rx` receiver, so it cannot be
    // rebuilt after a panic. A returned `run()` is fatal for uploads and must be
    // loud rather than silent.
    let manager_db = db.clone();
    tokio::spawn(async move {
        let mut manager = StorageManager::new(rx, manager_db, config_copy);
        tracing::debug!("running manager");
        manager.run().await;
        tracing::error!("storage manager loop exited; uploads will no longer be processed");
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

    let app_state = Arc::new(AppState::new(db, config.clone(), tx));

    // Best-effort cache warm-up. Spawned (not awaited) so a slow or huge
    // storage tree never delays serving; never fails startup either — every
    // error inside is logged and swallowed by the daemon itself.
    media_warmer::spawn(app_state.clone());

    let server = Server::build_server(app_state);

    let cert_store = CertStore::new(&config.certs_dir);
    cert_store
        .ensure_dir()
        .await
        .unwrap_or_else(|e| die(format!("failed to create CERTS_DIR: {e}")));
    let has_certs = cert_store.load_cert().await.ok().flatten().is_some()
        && cert_store.load_key().await.ok().flatten().is_some();
    let tls_mode = !plain_http && (config.tls_hostname.is_some() || has_certs);

    if tls_mode {
        let identity =
            config.tls_identity().unwrap_or_else(|e| die(format!("invalid TLS_HOSTNAME: {e}")));
        // One slot shared by startup issuance, the renewal task and the public
        // IP watcher, so a new address is picked up without a restart.
        let identity_slot = identity.clone().map(shared_identity);

        // Both the port-80 redirect and the startup banner point here, so it has
        // to be an address the certificate actually covers: an ACME certificate
        // carries only the TLS identity, never 127.0.0.1.
        let https_base = identity.as_ref().map_or_else(
            || format!("https://127.0.0.1:{}", config.https_addr.port()),
            |id| sarca::tls::identity_base_url(id, config.https_addr.port()),
        );

        let challenges = ChallengeStore::default();
        let acme_task =
            spawn_acme_http_listener(config.acme_http_addr, challenges.clone(), https_base.clone());

        // Give the ACME listener time to bind before http-01 validation.
        tokio::time::sleep(Duration::from_millis(100)).await;

        if acme_enabled(&config) {
            // Issuance is not on the boot path: http-01 validation can take
            // minutes, and blocking here meant a slow CA delayed serving and
            // logged a scary fallback warning. The renewal task below runs the
            // first attempt immediately and hot-reloads the certificate when it
            // lands; until then we serve the stored or self-signed material.
            tracing::info!("ACME enabled; first issuance runs in the background");
        } else {
            tracing::info!(
                "ACME disabled (SARCA_ACME=0 or empty ACME_DIRECTORY); using stored/self-signed TLS"
            );
        }

        let material = load_or_generate_material(&cert_store, identity.as_ref())
            .await
            .unwrap_or_else(|e| die(format!("failed to load TLS material: {e}")));

        let runtime = new_runtime(
            config.https_addr,
            config.acme_http_addr,
            &material,
            https_base,
            challenges,
        );

        if acme_enabled(&config) {
            if let Some(slot) = identity_slot {
                spawn_renewal_task(
                    InstantAcmeIssuer::from_parts(
                        config.acme_directory.clone(),
                        config.acme_http_addr,
                        slot.clone(),
                        runtime.challenges.clone(),
                        &cert_store.clone(),
                        config.acme_root_ca.as_ref().map(std::path::PathBuf::from),
                    ),
                    cert_store.clone(),
                    runtime.clone(),
                )
                .await;
                // The identity is an address we guessed, not one an operator
                // pinned, so keep watching it and re-issue when it moves.
                if identity_auto_detected {
                    spawn_public_ip_watch(slot, runtime.clone());
                }
            }
        }

        tracing::info!(
            "TLS mode: HTTPS {} (TCP+H3), ACME http://{}",
            config.https_addr,
            config.acme_http_addr
        );
        server.run_tls(runtime, Some(acme_task)).await;
    } else {
        if plain_http {
            tracing::info!("SARCA_PLAIN_HTTP=1 — plain HTTP on PORT (dev/e2e escape hatch)");
        } else {
            tracing::info!("no TLS identity or certs — plain HTTP on PORT");
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
