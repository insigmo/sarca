use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::types::{Binding, BindingMode};

#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub relative_path: String,
    pub size: i64,
    pub mtime_ms: i64,
    pub content_hash: Option<String>,
    pub remote_file_id: Option<Uuid>,
    pub last_cursor: i64,
}

/// Local SQLite index. `Connection` is wrapped in a mutex so the engine can be
/// shared across Tauri's async runtime (`Send + Sync`).
pub struct LocalIndex {
    conn: Mutex<Connection>,
}

impl LocalIndex {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("open index {}", path.display()))?;
        let idx = Self {
            conn: Mutex::new(conn),
        };
        idx.migrate()?;
        Ok(idx)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("sync index mutex poisoned"))
    }

    fn migrate(&self) -> Result<()> {
        self.lock()?.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS bindings (
              id TEXT PRIMARY KEY,
              storage_id TEXT NOT NULL,
              remote_root TEXT NOT NULL,
              local_path TEXT NOT NULL,
              mode TEXT NOT NULL,
              enabled INTEGER NOT NULL DEFAULT 1,
              cursor INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS entries (
              binding_id TEXT NOT NULL,
              relative_path TEXT NOT NULL,
              size INTEGER NOT NULL,
              mtime_ms INTEGER NOT NULL,
              content_hash TEXT,
              remote_file_id TEXT,
              last_cursor INTEGER NOT NULL DEFAULT 0,
              PRIMARY KEY (binding_id, relative_path)
            );
            CREATE TABLE IF NOT EXISTS conflicts (
              binding_id TEXT NOT NULL,
              relative_path TEXT NOT NULL,
              local_hash TEXT,
              remote_hash TEXT,
              PRIMARY KEY (binding_id, relative_path)
            );
            "#,
        )?;
        Ok(())
    }

    pub fn upsert_binding(&self, b: &Binding) -> Result<()> {
        self.lock()?.execute(
            r#"
            INSERT INTO bindings (id, storage_id, remote_root, local_path, mode, enabled, cursor)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, COALESCE((SELECT cursor FROM bindings WHERE id = ?1), 0))
            ON CONFLICT(id) DO UPDATE SET
              storage_id = excluded.storage_id,
              remote_root = excluded.remote_root,
              local_path = excluded.local_path,
              mode = excluded.mode,
              enabled = excluded.enabled
            "#,
            params![
                b.id,
                b.storage_id.to_string(),
                b.remote_root,
                b.local_path,
                mode_str(b.mode),
                i64::from(b.enabled),
            ],
        )?;
        Ok(())
    }

    pub fn list_bindings(&self) -> Result<Vec<Binding>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, storage_id, remote_root, local_path, mode, enabled FROM bindings ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            let mode: String = row.get(4)?;
            Ok(Binding {
                id: row.get(0)?,
                storage_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or(Uuid::nil()),
                remote_root: row.get(2)?,
                local_path: row.get(3)?,
                mode: parse_mode(&mode),
                enabled: row.get::<_, i64>(5)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn remove_binding(&self, id: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM entries WHERE binding_id = ?1", params![id])?;
        conn.execute("DELETE FROM conflicts WHERE binding_id = ?1", params![id])?;
        conn.execute("DELETE FROM bindings WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn set_binding_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let n = self.lock()?.execute(
            "UPDATE bindings SET enabled = ?2 WHERE id = ?1",
            params![id, i64::from(enabled)],
        )?;
        if n == 0 {
            anyhow::bail!("binding not found: {id}");
        }
        Ok(())
    }

    pub fn get_cursor(&self, binding_id: &str) -> Result<i64> {
        let v: Option<i64> = self
            .lock()?
            .query_row(
                "SELECT cursor FROM bindings WHERE id = ?1",
                params![binding_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.unwrap_or(0))
    }

    pub fn set_cursor(&self, binding_id: &str, cursor: i64) -> Result<()> {
        self.lock()?.execute(
            "UPDATE bindings SET cursor = ?2 WHERE id = ?1",
            params![binding_id, cursor],
        )?;
        Ok(())
    }

    pub fn get_entry(&self, binding_id: &str, relative_path: &str) -> Result<Option<IndexEntry>> {
        self.lock()?
            .query_row(
                r#"
                SELECT relative_path, size, mtime_ms, content_hash, remote_file_id, last_cursor
                FROM entries WHERE binding_id = ?1 AND relative_path = ?2
                "#,
                params![binding_id, relative_path],
                |row| {
                    Ok(IndexEntry {
                        relative_path: row.get(0)?,
                        size: row.get(1)?,
                        mtime_ms: row.get(2)?,
                        content_hash: row.get(3)?,
                        remote_file_id: row
                            .get::<_, Option<String>>(4)?
                            .and_then(|s| Uuid::parse_str(&s).ok()),
                        last_cursor: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn upsert_entry(&self, binding_id: &str, entry: &IndexEntry) -> Result<()> {
        self.lock()?.execute(
            r#"
            INSERT INTO entries (binding_id, relative_path, size, mtime_ms, content_hash, remote_file_id, last_cursor)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(binding_id, relative_path) DO UPDATE SET
              size = excluded.size,
              mtime_ms = excluded.mtime_ms,
              content_hash = excluded.content_hash,
              remote_file_id = excluded.remote_file_id,
              last_cursor = excluded.last_cursor
            "#,
            params![
                binding_id,
                entry.relative_path,
                entry.size,
                entry.mtime_ms,
                entry.content_hash,
                entry.remote_file_id.map(|u| u.to_string()),
                entry.last_cursor,
            ],
        )?;
        Ok(())
    }

    pub fn delete_entry(&self, binding_id: &str, relative_path: &str) -> Result<()> {
        self.lock()?.execute(
            "DELETE FROM entries WHERE binding_id = ?1 AND relative_path = ?2",
            params![binding_id, relative_path],
        )?;
        Ok(())
    }

    pub fn list_entry_paths(&self, binding_id: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT relative_path FROM entries WHERE binding_id = ?1")?;
        let rows = stmt.query_map(params![binding_id], |r| r.get(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn add_conflict(
        &self,
        binding_id: &str,
        relative_path: &str,
        local_hash: Option<&str>,
        remote_hash: Option<&str>,
    ) -> Result<()> {
        self.lock()?.execute(
            r#"
            INSERT INTO conflicts (binding_id, relative_path, local_hash, remote_hash)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(binding_id, relative_path) DO UPDATE SET
              local_hash = excluded.local_hash,
              remote_hash = excluded.remote_hash
            "#,
            params![binding_id, relative_path, local_hash, remote_hash],
        )?;
        Ok(())
    }

    pub fn clear_conflict(&self, binding_id: &str, relative_path: &str) -> Result<()> {
        self.lock()?.execute(
            "DELETE FROM conflicts WHERE binding_id = ?1 AND relative_path = ?2",
            params![binding_id, relative_path],
        )?;
        Ok(())
    }

    pub fn conflict_count(&self, binding_id: &str) -> Result<usize> {
        let n: i64 = self.lock()?.query_row(
            "SELECT COUNT(*) FROM conflicts WHERE binding_id = ?1",
            params![binding_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn default_path(data_dir: &Path) -> PathBuf {
        data_dir.join("sync-index.sqlite")
    }
}

fn mode_str(mode: BindingMode) -> &'static str {
    match mode {
        BindingMode::Sync => "sync",
        BindingMode::AutoUpload => "auto_upload",
        BindingMode::FolderUpload => "folder_upload",
    }
}

fn parse_mode(s: &str) -> BindingMode {
    match s {
        "auto_upload" => BindingMode::AutoUpload,
        "folder_upload" => BindingMode::FolderUpload,
        _ => BindingMode::Sync,
    }
}

#[allow(dead_code)]
pub fn mtime_ms_from_system(mtime: std::time::SystemTime) -> i64 {
    mtime
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[allow(dead_code)]
pub fn utc_from_mtime_ms(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_binding_enabled_preserves_entries() {
        let dir = tempfile::tempdir().unwrap();
        let idx = LocalIndex::open(&dir.path().join("sync-index.sqlite")).unwrap();
        let id = "b1".to_string();
        let sid = uuid::Uuid::new_v4();
        idx.upsert_binding(&crate::types::Binding {
            id: id.clone(),
            storage_id: sid,
            remote_root: "Camera".into(),
            local_path: "/tmp/pics".into(),
            mode: crate::types::BindingMode::AutoUpload,
            enabled: true,
        })
        .unwrap();
        idx.upsert_entry(
            &id,
            &IndexEntry {
                relative_path: "a.jpg".into(),
                size: 10,
                mtime_ms: 1,
                content_hash: Some("abc".into()),
                remote_file_id: None,
                last_cursor: 0,
            },
        )
        .unwrap();

        idx.set_binding_enabled(&id, false).unwrap();
        let b = idx
            .list_bindings()
            .unwrap()
            .into_iter()
            .find(|x| x.id == id)
            .unwrap();
        assert!(!b.enabled);
        assert!(idx.get_entry(&id, "a.jpg").unwrap().is_some());

        idx.set_binding_enabled(&id, true).unwrap();
        let b = idx
            .list_bindings()
            .unwrap()
            .into_iter()
            .find(|x| x.id == id)
            .unwrap();
        assert!(b.enabled);
        assert!(idx.get_entry(&id, "a.jpg").unwrap().is_some());
    }
}
