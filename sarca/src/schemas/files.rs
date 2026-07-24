use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct UploadParams {
    pub path: String,
    pub folder_name: String,
}

pub struct InFolderSchema {
    pub storage_id: Uuid,
    pub parent_path: String,
    pub folder_name: String,
}

impl InFolderSchema {
    pub fn new(storage_id: Uuid, parent_path: String, folder_name: String) -> Self {
        Self {
            storage_id,
            parent_path,
            folder_name,
        }
    }
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub search_path: Option<String>,
    /// When true (or "1"), serve with Content-Disposition: inline for in-app preview.
    #[serde(default)]
    pub inline: Option<String>,
}

#[derive(Deserialize)]
pub struct RenameSchema {
    /// Full old path (alternative to `path` + `new_name`)
    pub old_path: Option<String>,
    /// Full new path (alternative to `path` + `new_name`)
    pub new_path: Option<String>,
    /// Current path when using `new_name`
    pub path: Option<String>,
    /// New basename when using `path`
    pub new_name: Option<String>,
}

#[derive(Deserialize)]
pub struct MoveSchema {
    pub path: String,
    pub destination_folder: String,
}

#[derive(Deserialize)]
pub struct RestoreTrashSchema {
    pub path: String,
    /// `replace` | `rename` when a live file already exists at the path.
    pub on_conflict: Option<String>,
}

#[derive(Deserialize)]
pub struct TrashListQuery {
    /// Optional folder prefix inside trash (no leading slash).
    pub path: Option<String>,
}
