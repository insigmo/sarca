use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct FileSyncEvent {
    pub id: i64,
    pub storage_id: Uuid,
    pub file_id: Option<Uuid>,
    pub path: String,
    pub op: String,
    pub size: Option<i64>,
    pub is_file: bool,
    pub content_hash: Option<String>,
    pub source_mtime: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct SyncSnapshotEntry {
    pub file_id: Uuid,
    pub path: String,
    pub size: i64,
    pub is_file: bool,
    pub content_hash: Option<String>,
    pub source_mtime: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}
