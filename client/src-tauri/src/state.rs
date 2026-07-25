use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use sarca_sync::{
    Binding, BindingMode, KeepBothPrompt, SarcaApi, SyncEngine, SyncEngineConfig,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ServerConfig {
    pub base_url: String,
    pub access_token: String,
}

pub struct AppSyncState {
    pub engine: Arc<SyncEngine>,
    pub server: Arc<Mutex<ServerConfig>>,
    data_dir: PathBuf,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl AppSyncState {
    pub fn new(app: &AppHandle) -> Result<Self> {
        let data_dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from(".").join("sarca-client-data"));
        fs::create_dir_all(&data_dir)?;

        let server = load_server_config(&data_dir);
        let api = Arc::new(RwLock::new(SarcaApi::new(&server.base_url, &server.access_token)));
        let config = SyncEngineConfig {
            poll_interval: Duration::from_secs(30),
            api,
            data_dir: data_dir.clone(),
        };
        let engine = Arc::new(SyncEngine::open(config, Arc::new(KeepBothPrompt))?);
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);

        Ok(Self {
            engine,
            server: Arc::new(Mutex::new(server)),
            data_dir,
            shutdown_tx,
        })
    }

    pub fn start_background_loop(&self) {
        let engine = self.engine.clone();
        let rx = self.shutdown_tx.subscribe();
        tauri::async_runtime::spawn(async move {
            engine.run_loop(rx).await;
        });
    }

    pub fn server_config_path(&self) -> PathBuf {
        self.data_dir.join("server.json")
    }

    pub async fn save_server(&self, cfg: &ServerConfig) -> Result<()> {
        let json = serde_json::to_string_pretty(cfg)?;
        fs::write(self.server_config_path(), json)?;
        *self.server.lock().await = cfg.clone();
        self.engine
            .set_credentials(cfg.base_url.clone(), cfg.access_token.clone())
            .await;
        Ok(())
    }
}

pub fn load_server_config(data_dir: &PathBuf) -> ServerConfig {
    let path = data_dir.join("server.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn parse_mode(mode: &str) -> BindingMode {
    match mode {
        "auto_upload" => BindingMode::AutoUpload,
        _ => BindingMode::Sync,
    }
}

pub fn new_binding(
    storage_id: &str,
    remote_root: String,
    local_path: String,
    mode: &str,
) -> Result<Binding> {
    Ok(Binding {
        id: Uuid::new_v4().to_string(),
        storage_id: Uuid::parse_str(storage_id)?,
        remote_root,
        local_path,
        mode: parse_mode(mode),
        enabled: true,
    })
}
