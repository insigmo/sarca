use serde::Serialize;

pub struct InStorage {
    pub name: String,
    pub primary_position: i16,
}

impl InStorage {
    pub fn new(name: String) -> Self {
        Self {
            name,
            primary_position: 1,
        }
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Storage {
    pub id: uuid::Uuid,
    pub name: String,
    pub primary_position: i16,
}

impl Storage {
    pub fn new(id: uuid::Uuid, name: String, primary_position: i16) -> Self {
        Self {
            id,
            name,
            primary_position,
        }
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct StorageWithInfo {
    pub id: uuid::Uuid,
    pub name: String,
    pub primary_position: i16,
    pub files_amount: i64,
    pub size: i64,
    pub has_dead_channel: bool,
}
