use serde::Deserialize;

#[derive(Deserialize)]
pub struct UploadBodySchema {
    pub result: UploadResultSchema,
}

#[derive(Deserialize)]
pub struct UploadResultSchema {
    pub message_id: i64,
    pub document: UploadSchema,
}

#[derive(Deserialize)]
pub struct UploadSchema {
    pub file_id: String,
}

/// Result of a successful upload/copy: the Telegram file id plus the message id
/// that holds it in the target chat (needed later for `copyMessage`).
#[derive(Debug, Clone)]
pub struct UploadOutcome {
    pub file_id: String,
    pub message_id: i64,
}

#[derive(Deserialize)]
pub struct DownloadBodySchema {
    pub result: DownloadSchema,
}

#[derive(Deserialize)]
pub struct DownloadSchema {
    pub file_path: String,
    pub file_size: Option<u64>,
}

#[derive(Deserialize)]
pub struct GetChatBodySchema {
    pub result: GetChatResultSchema,
}

#[derive(Deserialize)]
pub struct GetChatResultSchema {
    pub id: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
}

/// Minimal chat info resolved via `getChat`, used to auto-fill a channel's display name.
#[derive(Debug, Clone)]
pub struct ChatInfo {
    pub id: i64,
    pub title: String,
}

#[derive(Deserialize)]
pub struct CopyMessageBodySchema {
    pub result: CopyMessageResultSchema,
}

#[derive(Deserialize)]
pub struct CopyMessageResultSchema {
    pub message_id: i64,
}
