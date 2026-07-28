use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingMode {
    /// Two-way folder sync (legacy; UI no longer creates these).
    Sync,
    /// One-way photo/video upload (Camera gallery).
    AutoUpload,
    /// One-way upload of all files from a local folder.
    FolderUpload,
}

impl BindingMode {
    /// Camera gallery or any-folder auto-upload (no remote→local pull).
    pub fn is_upload_only(self) -> bool {
        matches!(self, Self::AutoUpload | Self::FolderUpload)
    }
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
    /// Candidates returned by discovery this tick.
    #[serde(default)]
    pub scanned: usize,
    /// Candidates that needed upload work after size/mtime filter.
    #[serde(default)]
    pub pending: usize,
    /// `scanned - pending` when the tick completed without a discovery error.
    #[serde(default)]
    pub already_synced: usize,
}

/// Derive scan honesty counters from discovery + filter sizes.
pub fn scan_counters(scanned: usize, pending: usize) -> (usize, usize, usize) {
    (scanned, pending, scanned.saturating_sub(pending))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_status_scan_fields_default_to_zero() {
        let s = SyncStatus {
            binding_id: "b".into(),
            ..Default::default()
        };
        assert_eq!(s.scanned, 0);
        assert_eq!(s.pending, 0);
        assert_eq!(s.already_synced, 0);
    }

    #[test]
    fn scan_counters_compute_already_synced() {
        assert_eq!(scan_counters(5, 2), (5, 2, 3));
        assert_eq!(scan_counters(0, 0), (0, 0, 0));
    }
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
