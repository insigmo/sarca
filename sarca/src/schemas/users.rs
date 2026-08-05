use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct InUser {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct UserOut {
    pub id: Uuid,
    pub email: String,
    pub email_verified: bool,
    pub is_superuser: bool,
    pub disabled: bool,
}

#[derive(Serialize)]
pub struct UserListSchema {
    pub users: Vec<UserOut>,
}

#[derive(Deserialize)]
pub struct ChangeOwnPassword {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Deserialize)]
pub struct SetPassword {
    pub new_password: String,
}

#[derive(Deserialize)]
pub struct SetDisabled {
    pub disabled: bool,
}

#[derive(Debug, Serialize)]
pub struct UserDirectoryEntry {
    pub id: Uuid,
    pub email: String,
}

#[derive(Serialize)]
pub struct UserDirectorySchema {
    pub users: Vec<UserDirectoryEntry>,
}
