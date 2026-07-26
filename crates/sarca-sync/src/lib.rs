//! Sarca client sync engine: local index, HTTP API, folder sync + auto-upload.

mod api;
mod hash;
mod index;
mod types;
pub mod vfs;

pub mod engine;

pub use api::{normalize_server_url, LoginResponse, SarcaApi};
pub use engine::{ConflictChoice, ConflictPrompt, KeepBothPrompt, SyncEngine, SyncEngineConfig};
pub use hash::sha256_file;
pub use index::LocalIndex;
pub use types::{
    Binding, BindingMode, ChangelogEvent, ChangelogResponse, SnapshotEntry, SnapshotResponse,
    SyncStatus,
};
pub use vfs::{UnsupportedVirtualDrive, VirtualDrive};
