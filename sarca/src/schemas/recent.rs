use serde::Deserialize;

#[derive(Deserialize)]
pub struct RecentPathSchema {
    pub path: String,
}
