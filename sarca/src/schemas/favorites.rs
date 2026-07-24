use serde::Deserialize;

#[derive(Deserialize)]
pub struct FavoritePathSchema {
    pub path: String,
}
