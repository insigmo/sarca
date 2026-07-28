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
}

#[derive(Serialize)]
pub struct UserListSchema {
    pub users: Vec<UserOut>,
}
