use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct TrashSettingsSchema {
    pub retention_days: i32,
}
