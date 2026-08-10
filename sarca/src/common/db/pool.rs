use std::{path::Path, time::Duration};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

/// How long `SQLite` waits for a competing writer before returning `SQLITE_BUSY`.
///
/// Deliberately larger than the pool's acquire timeout: a writer that waits its
/// turn is healthy, a writer that gives up with `database is locked` is not.
/// The 5s value this replaced turned every slow batch into a hard error.
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// Connections reserved for the background loops (storage manager, replication,
/// channel health, trash purge, storage purge) on top of the HTTP workers.
///
/// Without this headroom `max_connections == WORKERS`, and the loops starved the
/// request handlers until they failed with `pool timed out while waiting for an
/// open connection`.
pub const BACKGROUND_CONNECTIONS: u32 = 6;

/// Open (creating if needed) the `SQLite` metadata database at `path`.
///
/// Every pooled connection gets `foreign_keys=ON`, `journal_mode=WAL` and a
/// `busy_timeout`, so concurrent HTTP handlers and background workers can share
/// one pool without tripping over the single writer lock.
pub async fn get_pool(
    path: &str,
    max_connection: u32,
    timeout: Duration,
) -> Result<SqlitePool, String> {
    create_parent_dir(path).await?;

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(BUSY_TIMEOUT)
        // Keep the WAL from growing unbounded between checkpoints on a busy
        // VPS, and keep sorting/temp tables off the disk.
        .pragma("wal_autocheckpoint", "1000")
        .pragma("cache_size", "-32000")
        .pragma("temp_store", "MEMORY");

    let connect = SqlitePoolOptions::new()
        .max_connections(max_connection.max(1))
        .min_connections(1)
        .acquire_timeout(timeout)
        .connect_with(options);

    match tokio::time::timeout(timeout, connect).await {
        Ok(Ok(db)) => {
            tracing::debug!("established connection with database");
            Ok(db)
        },
        Ok(Err(e)) => Err(format!("database connection failed ({path}): {e}")),
        Err(_) => {
            Err(format!("database connection timed out after {}s ({path})", timeout.as_secs()))
        },
    }
}

async fn create_parent_dir(path: &str) -> Result<(), String> {
    let Some(parent) = Path::new(path).parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| format!("failed to create database directory {}: {e}", parent.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creates_file_and_applies_pragmas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("sarca.sqlite");
        let path_str = path.to_str().unwrap();

        let pool = get_pool(path_str, 4, Duration::from_secs(5)).await.unwrap();

        assert!(path.exists(), "database file should be created");

        let journal_mode: String =
            sqlx::query_scalar("PRAGMA journal_mode").fetch_one(&pool).await.unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let foreign_keys: i64 =
            sqlx::query_scalar("PRAGMA foreign_keys").fetch_one(&pool).await.unwrap();
        assert_eq!(foreign_keys, 1);

        let busy_timeout: i64 =
            sqlx::query_scalar("PRAGMA busy_timeout").fetch_one(&pool).await.unwrap();
        assert_eq!(busy_timeout, BUSY_TIMEOUT.as_millis() as i64);

        pool.close().await;
    }
}
