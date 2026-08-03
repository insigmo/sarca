use chrono::{DateTime, Utc};

pub struct InDBUser {
    pub email: String,
    pub password_hash: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
}

impl InDBUser {
    pub fn new_password(email: String, password_hash: String) -> Self {
        Self {
            email,
            password_hash: Some(password_hash),
            email_verified_at: None,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct User {
    pub id: uuid::Uuid,
    pub email: String,
    pub password_hash: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
    /// Unix seconds; tokens whose `iat` is not newer are refused.
    #[sqlx(default)]
    pub sessions_valid_after: i64,
}

impl User {
    pub fn email_verified(&self) -> bool {
        self.email_verified_at.is_some()
    }

    /// Whether a token issued at `token_iat` (unix seconds) is still accepted.
    pub fn session_is_live(&self, token_iat: i64) -> bool {
        token_iat >= self.sessions_valid_after
    }
}
