use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Listing the source; items are being collected.
pub const JOB_SCANNING: &str = "scanning";
/// Items are being copied.
pub const JOB_RUNNING: &str = "running";
pub const JOB_PAUSED: &str = "paused";
pub const JOB_DONE: &str = "done";
pub const JOB_FAILED: &str = "failed";
pub const JOB_CANCELLED: &str = "cancelled";

pub const ITEM_PENDING: &str = "pending";
pub const ITEM_IN_PROGRESS: &str = "in_progress";
pub const ITEM_DONE: &str = "done";
/// Already in the storage at the target path with the same size; not copied.
pub const ITEM_SKIPPED: &str = "skipped";
pub const ITEM_FAILED: &str = "failed";

/// One import, as the runner reads it. Carries the source password, so it never
/// leaves the server: the API answers with `ImportJobSchema` instead.
#[derive(Debug, Clone, FromRow)]
pub struct ImportJob {
    pub id: Uuid,
    pub user_id: Uuid,
    pub storage_id: Uuid,
    pub source_kind: String,
    pub source_url: String,
    pub username: String,
    pub secret: Option<String>,
    pub target_path: String,
    pub scanned: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// A job with its item counts, for the API.
#[derive(Debug, Clone, FromRow)]
pub struct ImportJobSummary {
    pub id: Uuid,
    pub storage_id: Uuid,
    pub source_kind: String,
    pub source_url: String,
    pub username: String,
    pub has_secret: bool,
    pub target_path: String,
    pub status: String,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_files: i64,
    pub total_bytes: i64,
    pub done_files: i64,
    pub done_bytes: i64,
    pub skipped_files: i64,
    pub failed_files: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct ImportItem {
    pub id: i64,
    pub job_id: Uuid,
    pub rel_path: String,
    pub size: i64,
    pub source_mtime: Option<DateTime<Utc>>,
    pub status: String,
    pub attempts: i64,
    pub last_error: Option<String>,
}
