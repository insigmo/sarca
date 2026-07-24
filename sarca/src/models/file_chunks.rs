use crate::common::types::Position;

#[derive(Debug, sqlx::FromRow)]
pub struct FileChunk {
    pub id: uuid::Uuid,
    pub file_id: uuid::Uuid,
    pub position: Position,
}

impl FileChunk {
    pub fn new(id: uuid::Uuid, file_id: uuid::Uuid, position: Position) -> Self {
        Self {
            id,
            file_id,
            position,
        }
    }
}

/// Chunk plus a resolved Telegram file id for download from a specific channel replica.
#[derive(Debug, sqlx::FromRow)]
pub struct FileChunkWithReplica {
    pub id: uuid::Uuid,
    pub file_id: uuid::Uuid,
    pub position: Position,
    pub telegram_file_id: String,
    pub channel_id: uuid::Uuid,
}
