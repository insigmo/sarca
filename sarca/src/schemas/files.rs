use serde::{Deserialize, Serialize};
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
    /// `replace` | `rename` when destination basename already exists.
    pub on_conflict: Option<String>,
}

#[derive(Deserialize)]
pub struct CopySchema {
    pub path: String,
    pub destination_folder: String,
    /// `replace` | `rename` when destination basename already exists.
    pub on_conflict: Option<String>,
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

// A wire DTO, not a state machine: every flag here answers a separate question
// the UI and the sync client ask about one path, and folding them into enums
// would change the JSON for no gain at either end.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
pub struct FileInfoSchema {
    pub path: String,
    pub name: String,
    pub size: i64,
    pub is_file: bool,
    pub has_thumb: bool,
    pub is_uploaded: bool,
    /// True while this process is actually relaying the file onward to
    /// Telegram right now.
    ///
    /// `is_uploaded: false` alone cannot be read as "still working on it": a
    /// relay lives only in memory, so a row left behind by a crashed or killed
    /// process looks identical to one being worked on. Sync clients wait on a
    /// file they believe is relaying, and waiting on a row nobody owns is
    /// waiting forever — the file is never re-sent, and it is not in the
    /// storage either. This says which of the two it is.
    pub is_relaying: bool,
    pub content_type: Option<String>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    /// When the file was added to Sarca.
    pub added_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Original filesystem created time (client metadata), if known.
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Original filesystem modified time (client metadata), if known.
    pub modified_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Client-provided content hash when known (`sha256:...`).
    pub content_hash: Option<String>,
}
