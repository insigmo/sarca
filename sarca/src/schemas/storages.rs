use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    common::types::ChatId,
    models::{
        chunk_replicas::ReplicationStats,
        storage_channels::StorageChannel,
        storages::StorageWithInfo,
    },
};

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelInput {
    pub chat_id: ChatId,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub struct InStorageSchema {
    pub name: String,
    pub channels: Vec<ChannelInput>,
}

#[derive(Deserialize)]
pub struct UpdateStorageSchema {
    pub name: String,
}

#[derive(Serialize)]
pub struct StoragesListSchema {
    pub storages: Vec<StorageWithInfo>,
}

impl StoragesListSchema {
    pub fn new(storages: Vec<StorageWithInfo>) -> Self {
        Self {
            storages,
        }
    }
}

/// Full storage detail returned by `GET /storages/:id`: base storage fields plus
/// channels and replication stats used by the settings modal.
#[derive(Serialize)]
pub struct StorageDetailSchema {
    pub id: Uuid,
    pub name: String,
    pub primary_position: i16,
    pub has_dead_channel: bool,
    pub channels: Vec<StorageChannel>,
    pub replication: ReplicationStats,
    /// Bot bound to this storage (1:1), if any.
    pub bot: Option<StorageBotSchema>,
}

#[derive(Serialize)]
pub struct StorageBotSchema {
    pub id: Uuid,
    pub name: String,
    pub token_masked: String,
}

#[derive(Serialize)]
pub struct RefreshChannelsResultSchema {
    pub added: Vec<StorageChannel>,
    pub skipped_full: bool,
    pub skipped_in_use: Vec<ChatId>,
    pub channels: Vec<StorageChannel>,
    pub hint: Option<String>,
}

#[derive(Deserialize)]
pub struct SetStorageBotSchema {
    pub token: String,
}

#[derive(Deserialize)]
pub struct AddChannelSchema {
    pub chat_id: ChatId,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateChannelSchema {
    #[serde(default)]
    pub chat_id: Option<ChatId>,
    #[serde(default)]
    pub name: Option<String>,
}
