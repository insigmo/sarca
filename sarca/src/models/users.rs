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

    pub fn new_oauth(email: String, email_verified: bool) -> Self {
        Self {
            email,
            password_hash: None,
            email_verified_at: if email_verified { Some(Utc::now()) } else { None },
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct User {
    pub id: uuid::Uuid,
    pub email: String,
    pub password_hash: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
}

impl User {
    pub fn email_verified(&self) -> bool {
        self.email_verified_at.is_some()
    }
}
