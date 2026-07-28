use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
#[allow(dead_code)] // Queried shape reserved for admin/ops; worker uses ClaimedPurgeMessage.
pub struct StoragePurgeJob {
    pub id: Uuid,
    pub storage_id: Uuid,
    pub bot_token: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ClaimedPurgeMessage {
    pub id: i64,
    pub job_id: Uuid,
    pub chat_id: i64,
    pub message_id: i64,
    pub bot_token: String,
    pub attempts: i32,
}
