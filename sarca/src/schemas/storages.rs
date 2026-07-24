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
