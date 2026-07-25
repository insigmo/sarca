use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingMode {
    Sync,
    AutoUpload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub id: String,
    pub storage_id: Uuid,
    pub remote_root: String,
    pub local_path: String,
    pub mode: BindingMode,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncStatus {
    pub binding_id: String,
    pub cursor: i64,
    pub last_error: Option<String>,
    pub uploading: usize,
    pub downloading: usize,
    pub conflicts: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChangelogEvent {
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChangelogResponse {
    pub events: Vec<ChangelogEvent>,
    pub next_cursor: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SnapshotEntry {
    pub file_id: Uuid,
    pub path: String,
    pub size: i64,
    pub is_file: bool,
    pub content_hash: Option<String>,
    pub source_mtime: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SnapshotResponse {
    pub files: Vec<SnapshotEntry>,
    pub cursor: i64,
}
