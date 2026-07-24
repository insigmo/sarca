use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::share_links::ShareLink;

#[derive(Deserialize)]
pub struct CreateShareSchema {
    pub path: String,
    pub expires_at: Option<DateTime<Utc>>,
    /// Omit or null = no password.
    pub password: Option<String>,
}

#[derive(Deserialize)]
pub struct UnlockShareSchema {
    pub password: String,
}

#[derive(Deserialize)]
pub struct PublicTreeQuery {
    /// Path relative to the share root.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct ShareLinkSchema {
    pub id: Uuid,
    pub token: String,
    pub url_path: String,
    pub path: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub has_password: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl ShareLinkSchema {
    pub fn from_link(link: &ShareLink) -> Self {
        Self {
            id: link.id,
            token: link.token.clone(),
            url_path: format!("/s/{}", link.token),
            path: link.path.clone(),
            expires_at: link.expires_at,
            has_password: link.has_password(),
            created_at: link.created_at,
            revoked_at: link.revoked_at,
        }
    }
}

#[derive(Serialize)]
pub struct PublicShareMetaSchema {
    pub path: String,
    pub name: String,
    pub is_file: bool,
    pub size: i64,
    pub has_password: bool,
}

#[derive(Serialize)]
pub struct NeedPasswordSchema {
    pub need_password: bool,
}

impl NeedPasswordSchema {
    pub fn yes() -> Self {
        Self {
            need_password: true,
        }
    }
}
