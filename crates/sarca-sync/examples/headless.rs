//! Headless driver for the sync engine — used by the e2e suite to exercise
//! auto-upload / folder-upload without the Tauri client.
//!
//! ```text
//! cargo run -p sarca-sync --example headless -- \
//!   --server http://127.0.0.1:8000 --email a@b.c --password secret \
//!   --storage-id <uuid> --local /tmp/dcim --remote-root Camera \
//!   --mode auto_upload --ticks 2 --data-dir /tmp/sync-state
//! ```
//!
//! Prints one JSON object on stdout: bindings, per-binding statuses and the
//! transfer queue after the last tick.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{bail, Result};
use sarca_sync::{
    Binding, BindingMode, FsMediaSource, KeepBothPrompt, SarcaApi, SyncEngine, SyncEngineConfig,
};
use uuid::Uuid;

fn parse_args() -> HashMap<String, String> {
    let mut args = HashMap::new();
    let mut argv = std::env::args().skip(1);
    while let Some(key) = argv.next() {
        let Some(key) = key.strip_prefix("--") else {
            continue;
        };
        args.insert(key.to_owned(), argv.next().unwrap_or_default());
    }
    args
}

fn required(args: &HashMap<String, String>, key: &str) -> Result<String> {
    match args.get(key) {
        Some(value) if !value.is_empty() => Ok(value.clone()),
        _ => bail!("missing --{key}"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();

    let server = required(&args, "server")?;
    let email = required(&args, "email")?;
    let password = required(&args, "password")?;
    let storage_id: Uuid = required(&args, "storage-id")?.parse()?;
    let local = required(&args, "local")?;
    let remote_root = args.get("remote-root").cloned().unwrap_or_default();
    let mode = match args
        .get("mode")
        .map(String::as_str)
        .unwrap_or("auto_upload")
    {
        "auto_upload" => BindingMode::AutoUpload,
        "folder_upload" => BindingMode::FolderUpload,
        "sync" => BindingMode::Sync,
        other => bail!("unknown --mode {other}"),
    };
    let ticks: usize = args.get("ticks").and_then(|t| t.parse().ok()).unwrap_or(1);
    let data_dir = PathBuf::from(required(&args, "data-dir")?);
    let binding_id = args
        .get("binding-id")
        .cloned()
        .unwrap_or_else(|| "e2e-binding".to_owned());

    std::fs::create_dir_all(&data_dir)?;

    let login = SarcaApi::login(&server, &email, &password).await?;
    let api = SarcaApi::new(server.clone(), login.access_token.clone());

    let engine = SyncEngine::open(
        SyncEngineConfig {
            poll_interval: std::time::Duration::from_secs(60),
            api: Arc::new(tokio::sync::RwLock::new(api)),
            data_dir,
            media_source: Arc::new(FsMediaSource),
        },
        Arc::new(KeepBothPrompt),
    )?;

    engine.upsert_binding(&Binding {
        id: binding_id.clone(),
        storage_id,
        remote_root,
        local_path: local,
        mode,
        enabled: true,
    })?;

    let mut tick_errors: Vec<String> = Vec::new();
    for _ in 0..ticks {
        if let Err(e) = engine.tick().await {
            tick_errors.push(e.to_string());
        }
    }

    let statuses = engine.statuses().await;
    let transfers = engine.transfer_queue().await;
    let bindings = engine.list_bindings()?;

    println!(
        "{}",
        serde_json::json!({
            "binding_id": binding_id,
            "bindings": bindings,
            "statuses": statuses,
            "transfers": transfers,
            "tick_errors": tick_errors,
        })
    );

    Ok(())
}
