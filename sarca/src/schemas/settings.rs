use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct TrashSettingsSchema {
    pub retention_days: i32,
}

/// Password guard for a backup download or a restore upload. Optional: an
/// archive without one is plain gzip that anyone can open, including the bot
/// tokens inside it.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BackupPasswordSchema {
    #[serde(default)]
    pub password: Option<String>,
}

/// What a restore actually moved, so the operator can tell a real restore from
/// one that quietly did nothing.
#[derive(Debug, Serialize, Deserialize)]
pub struct RestoreResultSchema {
    /// Tables copied out of the archive.
    pub tables: usize,
    /// Rows written across those tables.
    pub rows: u64,
    /// Tables the archive carried that this build has no table for.
    pub skipped_tables: Vec<String>,
    /// Server-side path of the pre-restore copy of the old database, when one
    /// could be written.
    pub safety_copy: Option<String>,
}
