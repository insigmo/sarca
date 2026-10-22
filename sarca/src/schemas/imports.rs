use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::imports::{ImportItem, ImportJobSummary};

#[derive(Debug, Deserialize)]
pub struct CreateImportSchema {
    pub storage_id: Uuid,
    /// WebDAV address of the folder to copy, e.g.
    /// `https://cloud.example.com/remote.php/dav/files/alice/Photos/`.
    pub url: String,
    pub username: String,
    pub password: String,
    /// Folder in the storage the files land under; empty for the root.
    #[serde(default)]
    pub target_path: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct ResumeImportSchema {
    /// Replaces the stored source password when given.
    #[serde(default)]
    pub password: Option<String>,
}

/// A job as the API shows it. The password is never part of it; `has_password`
/// says whether the server still holds one.
#[derive(Debug, Serialize)]
pub struct ImportJobSchema {
    pub id: Uuid,
    pub storage_id: Uuid,
    pub source_kind: String,
    pub source_url: String,
    pub username: String,
    pub has_password: bool,
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

impl From<ImportJobSummary> for ImportJobSchema {
    fn from(s: ImportJobSummary) -> Self {
        Self {
            id: s.id,
            storage_id: s.storage_id,
            source_kind: s.source_kind,
            source_url: s.source_url,
            username: s.username,
            has_password: s.has_secret,
            target_path: s.target_path,
            status: s.status,
            last_error: s.last_error,
            created_at: s.created_at,
            updated_at: s.updated_at,
            completed_at: s.completed_at,
            total_files: s.total_files,
            total_bytes: s.total_bytes,
            done_files: s.done_files,
            done_bytes: s.done_bytes,
            skipped_files: s.skipped_files,
            failed_files: s.failed_files,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ImportItemSchema {
    pub path: String,
    pub size: i64,
    pub attempts: i64,
    pub error: Option<String>,
}

impl From<ImportItem> for ImportItemSchema {
    fn from(i: ImportItem) -> Self {
        Self {
            path: i.rel_path,
            size: i.size,
            attempts: i.attempts,
            error: i.last_error,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ImportDetailSchema {
    #[serde(flatten)]
    pub job: ImportJobSchema,
    /// Up to 100 files that could not be copied.
    pub failed_items: Vec<ImportItemSchema>,
}

#[derive(Debug, Serialize)]
pub struct ImportListSchema {
    pub imports: Vec<ImportJobSchema>,
}
