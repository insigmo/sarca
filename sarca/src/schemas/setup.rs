use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::common::types::ChatId;

#[derive(Debug, Serialize)]
pub struct SetupStatusSchema {
    pub has_storages: bool,
    pub uses_local_api: bool,
    pub local_api_ready: bool,
    pub local_api_skipped: bool,
    /// Show Phase A when true.
    pub needs_local_api_phase: bool,
    pub conf_writable: bool,
}

#[derive(Debug, Deserialize)]
pub struct LocalApiCredentialsSchema {
    pub api_id: String,
    pub api_hash: String,
}

#[derive(Debug, Serialize)]
pub struct LocalApiSaveResultSchema {
    pub saved_to_settings: bool,
    pub saved_to_conf: bool,
    pub restart_hint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LocalApiVerifySchema {
    pub ok: bool,
    pub uses_local_api: bool,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct BotTokenSchema {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct BotValidateSchema {
    pub bot_id: i64,
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct ChannelPollSchema {
    pub token: String,
    #[serde(default)]
    pub exclude_chat_ids: Vec<ChatId>,
}

#[derive(Debug, Serialize)]
pub struct ChannelPollResultSchema {
    pub found: bool,
    pub chat_id: Option<ChatId>,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetupCreateStorageSchema {
    pub name: String,
    pub token: String,
    pub chat_ids: Vec<ChatId>,
}

#[derive(Debug, Serialize)]
pub struct SetupCreateStorageResultSchema {
    pub id: Uuid,
    pub name: String,
}
