use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct ShareLink {
    pub id: Uuid,
    pub token: String,
    pub storage_id: Uuid,
    pub path: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub password_hash: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl ShareLink {
    pub fn has_password(&self) -> bool {
        self.password_hash.is_some()
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| exp <= Utc::now())
            .unwrap_or(false)
    }

    /// Unavailable to guests (revoked or past expiry).
    pub fn is_unavailable(&self) -> bool {
        self.is_revoked() || self.is_expired()
    }

    pub fn is_folder(&self) -> bool {
        self.path.ends_with('/')
    }
}
