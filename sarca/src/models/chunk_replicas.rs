use serde::Serialize;

pub const REPLICA_STATUS_PENDING: &str = "pending";
pub const REPLICA_STATUS_UPLOADED: &str = "uploaded";
pub const REPLICA_STATUS_FAILED: &str = "failed";

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ChunkReplica {
    pub id: uuid::Uuid,
    pub chunk_id: uuid::Uuid,
    pub channel_id: uuid::Uuid,
    pub telegram_file_id: Option<String>,
    pub telegram_message_id: Option<i64>,
    pub status: String,
}

impl ChunkReplica {
    pub fn new_uploaded(
        id: uuid::Uuid,
        chunk_id: uuid::Uuid,
        channel_id: uuid::Uuid,
        telegram_file_id: String,
        telegram_message_id: i64,
    ) -> Self {
        Self {
            id,
            chunk_id,
            channel_id,
            telegram_file_id: Some(telegram_file_id),
            telegram_message_id: Some(telegram_message_id),
            status: REPLICA_STATUS_UPLOADED.to_owned(),
        }
    }

    pub fn new_pending(id: uuid::Uuid, chunk_id: uuid::Uuid, channel_id: uuid::Uuid) -> Self {
        Self {
            id,
            chunk_id,
            channel_id,
            telegram_file_id: None,
            telegram_message_id: None,
            status: REPLICA_STATUS_PENDING.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ReplicationStats {
    pub pending: i64,
    pub uploaded: i64,
    pub failed: i64,
}
