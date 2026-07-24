use chrono::{DateTime, Utc};
use uuid::Uuid;

pub const PURPOSE_VERIFY: &str = "verify";
pub const PURPOSE_RESET: &str = "reset";

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
pub struct EmailToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub purpose: String,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
