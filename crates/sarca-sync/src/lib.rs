//! Sarca client sync engine: local index, HTTP API, folder sync + auto-upload.

mod api;
mod hash;
mod index;
mod scheduler;
mod types;
pub mod vfs;

pub mod engine;

pub use api::{
    authorization_header_value, normalize_server_url, LoginResponse, SarcaApi, StorageSummary,
};
pub use engine::{
    is_media_file, ConflictChoice, ConflictPrompt, KeepBothPrompt, SyncEngine, SyncEngineConfig,
};
pub use hash::sha256_file;
pub use index::LocalIndex;
pub use scheduler::BindingScheduler;
pub use types::{
    Binding, BindingMode, ChangelogEvent, ChangelogResponse, SnapshotEntry, SnapshotResponse,
    SyncStatus,
};
pub use vfs::{UnsupportedVirtualDrive, VirtualDrive};
