use serde::Deserialize;
use uuid::Uuid;

use crate::models::access::AccessType;

#[derive(Deserialize)]
pub struct GrantAccess {
    pub user_email: String,
    pub access_type: AccessType,
}

#[derive(Deserialize)]
pub struct RestrictAccess {
    pub user_id: Uuid,
}
