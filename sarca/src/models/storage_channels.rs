use serde::Serialize;

use crate::common::types::ChatId;

pub const CHANNEL_STATUS_ACTIVE: &str = "active";
pub const CHANNEL_STATUS_DEAD: &str = "dead";

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StorageChannel {
    pub id: uuid::Uuid,
    pub storage_id: uuid::Uuid,
    pub position: i16,
    pub chat_id: ChatId,
    pub name: String,
    pub status: String,
}

impl StorageChannel {
    pub fn is_active(&self) -> bool {
        self.status == CHANNEL_STATUS_ACTIVE
    }

    pub fn is_dead(&self) -> bool {
        self.status == CHANNEL_STATUS_DEAD
    }
}

#[derive(Debug, Clone)]
pub struct InStorageChannel {
    pub storage_id: uuid::Uuid,
    pub position: i16,
    pub chat_id: ChatId,
    pub name: String,
    pub status: String,
}

impl InStorageChannel {
    pub fn active(storage_id: uuid::Uuid, position: i16, chat_id: ChatId, name: String) -> Self {
        Self {
            storage_id,
            position,
            chat_id,
            name,
            status: CHANNEL_STATUS_ACTIVE.to_owned(),
        }
    }
}
