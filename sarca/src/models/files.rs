use serde::Serialize;

/// Extensions treated as video for progressive upload chunking.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "webm", "mkv", "mov", "m4v", "avi", "mpeg", "mpg", "ogv", "3gp",
];

/// True when `path` looks like a video (by extension), or `content_type` is `video/*`.
pub fn is_video(path: &str, content_type: Option<&str>) -> bool {
    if let Some(ct) = content_type {
        let ct = ct.trim().to_ascii_lowercase();
        if ct.starts_with("video/") {
            return true;
        }
    }
    path.rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| VIDEO_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

pub struct InFile {
    pub path: String,
    pub size: i64,
    pub storage_id: uuid::Uuid,
    /// Telegram chunk size used for this file; `None` for folders / legacy.
    pub chunk_size_bytes: Option<i64>,
}

impl InFile {
    pub fn new(path: String, size: i64, storage_id: uuid::Uuid) -> Self {
        Self {
            path,
            size,
            storage_id,
            chunk_size_bytes: None,
        }
    }

    pub fn with_chunk_size(mut self, chunk_size_bytes: i64) -> Self {
        self.chunk_size_bytes = Some(chunk_size_bytes);
        self
    }
}

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
pub struct File {
    pub id: uuid::Uuid,
    pub path: String,
    pub size: i64,
    pub storage_id: uuid::Uuid,
    pub is_uploaded: bool,
    pub thumb_telegram_file_id: Option<String>,
    /// Telegram chunk size used at upload; `None` for pre-feature / folder rows.
    pub chunk_size_bytes: Option<i64>,
}

impl File {
    pub fn new(
        id: uuid::Uuid,
        path: String,
        size: i64,
        storage_id: uuid::Uuid,
        is_uploaded: bool,
        chunk_size_bytes: Option<i64>,
    ) -> Self {
        Self {
            id,
            path,
            size,
            storage_id,
            is_uploaded,
            thumb_telegram_file_id: None,
            chunk_size_bytes,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct DBFSElement {
    pub name: String,
    pub size: i64,
    pub is_file: bool,
    pub has_thumb: bool,
}

#[derive(Debug, sqlx::FromRow, Serialize)]
pub struct FSElement {
    pub path: String,
    pub name: String,
    pub size: i64,
    pub is_file: bool,
    pub has_thumb: bool,
}

#[derive(Debug, sqlx::FromRow, Serialize)]
pub struct SearchFSElement {
    pub path: String,
    pub is_file: bool,
}
