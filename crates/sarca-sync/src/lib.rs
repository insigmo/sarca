//! Sarca client sync engine: local index, HTTP API, folder sync + auto-upload.

mod api;
mod candidate;
mod hash;
mod index;
mod media_source;
mod scheduler;
mod transfer;
mod types;
pub mod vfs;

pub mod engine;

pub use api::{
    authorization_header_value, normalize_server_url, LoginResponse, SarcaApi, StorageSummary,
};
pub use candidate::{collect_fs_candidates, is_media_file, strip_dcim_prefix, LocalCandidate};
pub use engine::{
    select_pending_uploads, ConflictChoice, ConflictPrompt, KeepBothPrompt, SyncEngine,
    SyncEngineConfig,
};
pub use hash::sha256_file;
pub use index::LocalIndex;
pub use media_source::{FsMediaSource, LocalMediaSource};
pub use scheduler::BindingScheduler;
pub use transfer::{
    TransferDirection, TransferItem, TransferQueueSnapshot, TransferStatus,
};
pub use types::{
    scan_counters, Binding, BindingMode, ChangelogEvent, ChangelogResponse, SnapshotEntry,
    SnapshotResponse, SyncStatus,
};
pub use vfs::{UnsupportedVirtualDrive, VirtualDrive};
