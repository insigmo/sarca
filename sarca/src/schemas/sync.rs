use serde::{Deserialize, Serialize};

use crate::models::file_sync_events::{FileSyncEvent, SyncSnapshotEntry};

#[derive(Debug, Deserialize)]
pub struct ChangelogQuery {
    /// Last seen event id; omit or 0 for the beginning.
    pub cursor: Option<i64>,
    /// Max events to return (default 500, max 2000).
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ChangelogResponse {
    pub events: Vec<FileSyncEvent>,
    pub next_cursor: i64,
    pub has_more: bool,
}

impl ChangelogResponse {
    pub fn new(events: Vec<FileSyncEvent>, requested_limit: i64) -> Self {
        let has_more = events.len() as i64 >= requested_limit;
        let next_cursor = events.last().map_or(0, |e| e.id);
        Self {
            events,
            next_cursor,
            has_more,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SnapshotResponse {
    pub files: Vec<SyncSnapshotEntry>,
    pub cursor: i64,
}
