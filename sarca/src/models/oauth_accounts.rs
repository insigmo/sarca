use chrono::{DateTime, Utc};
use uuid::Uuid;

pub const PROVIDER_GOOGLE: &str = "google";
pub const PROVIDER_GITHUB: &str = "github";

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
pub struct OAuthAccount {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: String,
    pub provider_user_id: String,
    pub created_at: DateTime<Utc>,
}
