use std::path::PathBuf;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct UploadParams {
    pub path: String,
    pub folder_name: String,
}

pub struct InFileSchema {
    pub storage_id: Uuid,
    pub path: String,
    pub size: i64,
    pub file_path: PathBuf,
}

impl InFileSchema {
    pub fn new(storage_id: Uuid, path: String, file_path: PathBuf, size: i64) -> Self {
        Self {
            storage_id,
            path,
            size,
            file_path,
        }
    }
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
