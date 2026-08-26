//! Backup and restore of the `SQLite` metadata database.
//!
//! The database *is* the deployment: settings, storages and their bot tokens,
//! channels, users and access grants, and the whole file tree with the Telegram
//! message ids behind every chunk. Nothing else on disk is irreplaceable —
//! thumbnails and previews are re-derived on demand, and the file bytes
//! themselves live in Telegram. So a single archive of this one file is enough
//! to stand the same instance up somewhere else and keep browsing the same
//! storages.
//!
//! Snapshots are taken with `VACUUM INTO`, which produces a consistent copy
//! while the server keeps serving; a plain file copy of a live WAL database
//! would not be.
//!
//! Restore replaces the *contents* of the live database, table by table, inside
//! one transaction on a dedicated connection. Swapping the file underneath the
//! open pool is not an option: every pooled connection holds the old inode, and
//! Windows will not rename over a file that is open at all. Copying rows in
//! place also survives the version gap between the machine that wrote the backup
//! and the one restoring it — columns are matched by name, so an older archive
//! restores into today's schema and simply leaves new columns at their default.

use std::{
    collections::HashSet,
    fs::File,
    io::{self, BufReader, BufWriter},
    path::{Path, PathBuf},
    time::Duration,
};

use sqlx::{
    ConnectOptions,
    Connection,
    SqliteConnection,
    SqlitePool,
    sqlite::SqliteConnectOptions,
};
use uuid::Uuid;

use crate::{
    common::backup_archive,
    config::Config,
    errors::{SarcaError, SarcaResult},
    schemas::settings::RestoreResultSchema,
    startup::{create_superuser, delete_orphan_storage_workers, init_db},
};

/// Extension (and download suffix) for a backup archive.
pub const BACKUP_EXTENSION: &str = "sarcabak";

/// Tables a file must contain before we are willing to wipe the live database
/// for it. A valid `SQLite` file that is not a Sarca database would otherwise
/// restore "successfully" into an empty instance.
const REQUIRED_TABLES: [&str; 4] = ["users", "storages", "storage_workers", "files"];

/// Pre-restore safety copies kept in `WORK_DIR/backups`. Three is enough to walk
/// back a bad restore without letting copies of the whole database pile up.
const SAFETY_COPIES_KEPT: usize = 3;

/// Long enough that a restore never loses to an ordinary request holding the
/// write lock, short enough that a wedged one still reports rather than hangs.
const RESTORE_BUSY_TIMEOUT: Duration = Duration::from_secs(60);

/// A file removed when the handle is dropped — including on a client
/// disconnect halfway through the download, which is exactly when a plain
/// "delete after streaming" leaks.
#[derive(Debug)]
pub struct ScratchFile(PathBuf);

impl ScratchFile {
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let path = std::mem::take(&mut self.0);
        tokio::spawn(async move {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                if e.kind() != io::ErrorKind::NotFound {
                    tracing::warn!("failed to remove scratch file {}: {e}", path.display());
                }
            }
        });
    }
}

/// A finished archive waiting to be streamed to the client.
#[derive(Debug)]
pub struct BackupArtifact {
    pub file: ScratchFile,
    /// Suggested download name, e.g. `sarca-backup-20260826-141233.sarcabak`.
    pub filename: String,
    pub size_bytes: u64,
}

pub struct BackupService;

impl BackupService {
    /// Snapshot the database and wrap it in a `.sarcabak` archive.
    ///
    /// `password` is optional: without one the archive is plain gzip, which
    /// anyone who gets hold of the file can read — including every storage's
    /// bot token.
    pub async fn create(
        db: &SqlitePool,
        work_dir: &str,
        password: Option<&str>,
    ) -> SarcaResult<BackupArtifact> {
        let encrypted = password.is_some();
        let dir = scratch_dir(work_dir).await?;
        let snapshot = ScratchFile::new(dir.join(format!("snapshot-{}.sqlite", Uuid::new_v4())));
        snapshot_into(db, snapshot.path()).await?;

        let archive =
            ScratchFile::new(dir.join(format!("archive-{}.{BACKUP_EXTENSION}", Uuid::new_v4())));

        let (source, target) = (snapshot.path().to_path_buf(), archive.path().to_path_buf());
        let password = password.map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            let reader = BufReader::new(File::open(&source)?);
            let mut writer = BufWriter::new(File::create(&target)?);
            backup_archive::encode(reader, &mut writer, password.as_deref())?;
            io::Write::flush(&mut writer)
        })
        .await
        .map_err(|e| internal("backup encode task", &e))?
        .map_err(|e| internal("backup encode", &e))?;

        drop(snapshot);

        let size_bytes = tokio::fs::metadata(archive.path())
            .await
            .map_err(|e| internal("backup size", &e))?
            .len();

        tracing::info!(size_bytes, encrypted, "created database backup");

        Ok(BackupArtifact {
            filename: format!("sarca-backup-{}.{BACKUP_EXTENSION}", timestamp()),
            file: archive,
            size_bytes,
        })
    }

    /// Replace the live database's contents with those of `archive`.
    ///
    /// A safety copy of the current database is written to `WORK_DIR/backups`
    /// first, so a restore of the wrong file is recoverable from the server's
    /// own disk without needing the previous archive.
    pub async fn restore(
        db: &SqlitePool,
        config: &Config,
        archive: &Path,
        password: Option<&str>,
    ) -> SarcaResult<RestoreResultSchema> {
        let dir = scratch_dir(config.work_dir.as_str()).await?;
        let staged = ScratchFile::new(dir.join(format!("staged-{}.sqlite", Uuid::new_v4())));

        decode_archive(archive, staged.path(), password).await?;
        validate_snapshot(staged.path()).await?;

        let safety_copy = write_safety_copy(db, &dir).await;

        let outcome = apply_snapshot(&config.sqlite_path, staged.path()).await?;
        drop(staged);

        // The restored rows came from whatever schema that instance ran, and
        // the configured superuser may not exist in them at all. Re-running the
        // boot-time fixups is what keeps the operator able to log back in.
        init_db(db).await;
        delete_orphan_storage_workers(db).await;
        create_superuser(db, config).await;

        tracing::warn!(
            tables = outcome.tables,
            rows = outcome.rows,
            "database restored from backup; existing sessions are no longer valid"
        );

        Ok(RestoreResultSchema {
            tables: outcome.tables,
            rows: outcome.rows,
            skipped_tables: outcome.skipped_tables,
            safety_copy,
        })
    }
}

/// Rows and tables actually moved by [`apply_snapshot`].
struct RestoreOutcome {
    tables: usize,
    rows: u64,
    /// Tables the archive carried that this build has no table for — data from a
    /// newer Sarca that today's schema cannot hold.
    skipped_tables: Vec<String>,
}

fn timestamp() -> String {
    chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

fn internal(what: &str, e: &dyn std::fmt::Display) -> SarcaError {
    tracing::error!("{what} failed: {e}");
    SarcaError::Unknown
}

/// `WORK_DIR/backups`, created on demand. Also where pre-restore safety copies
/// land, so an operator has one place to look.
pub async fn scratch_dir(work_dir: &str) -> SarcaResult<PathBuf> {
    let dir = Path::new(work_dir).join("backups");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| internal("creating WORK_DIR/backups", &e))?;
    Ok(dir)
}

/// Consistent copy of the live database, taken without stopping the server.
async fn snapshot_into(db: &SqlitePool, target: &Path) -> SarcaResult<()> {
    // VACUUM INTO refuses an existing target; a stale file from a crashed run
    // would otherwise fail every backup from then on.
    match tokio::fs::remove_file(target).await {
        Ok(()) => {},
        Err(e) if e.kind() == io::ErrorKind::NotFound => {},
        Err(e) => return Err(internal("clearing snapshot target", &e)),
    }

    // No bind parameter: `VACUUM INTO` takes a literal in every SQLite build we
    // ship against. The path is ours (a UUID under WORK_DIR), and the quote
    // doubling keeps it a literal even so.
    let literal = target.to_string_lossy().replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{literal}'"))
        .execute(db)
        .await
        .map_err(|e| internal("VACUUM INTO", &e))?;
    Ok(())
}

async fn decode_archive(archive: &Path, target: &Path, password: Option<&str>) -> SarcaResult<()> {
    let (archive, target) = (archive.to_path_buf(), target.to_path_buf());
    let password = password.map(str::to_owned);

    tokio::task::spawn_blocking(move || {
        let reader = BufReader::new(File::open(&archive)?);
        let mut writer = BufWriter::new(File::create(&target)?);
        backup_archive::decode(reader, &mut writer, password.as_deref())?;
        io::Write::flush(&mut writer)
    })
    .await
    .map_err(|e| internal("backup decode task", &e))?
    .map_err(|e| {
        match e.kind() {
            io::ErrorKind::PermissionDenied => SarcaError::BackupPasswordRequired,
            io::ErrorKind::InvalidData => SarcaError::InvalidBackupFile(e.to_string()),
            _ => internal("backup decode", &e),
        }
    })
}

/// Refuse anything that is not an intact Sarca database *before* the live one
/// is touched.
async fn validate_snapshot(snapshot: &Path) -> SarcaResult<()> {
    let mut conn = SqliteConnectOptions::new()
        .filename(snapshot)
        .create_if_missing(false)
        .read_only(true)
        .connect()
        .await
        .map_err(|e| {
            SarcaError::InvalidBackupFile(format!("backup does not contain a database ({e})"))
        })?;

    let check: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&mut conn)
        .await
        .map_err(|e| SarcaError::InvalidBackupFile(format!("backup is unreadable ({e})")))?;
    if !check.eq_ignore_ascii_case("ok") {
        let _ = conn.close().await;
        return Err(SarcaError::InvalidBackupFile("backup database is corrupted".to_owned()));
    }

    let tables = table_names(&mut conn, "main").await?;
    let missing: Vec<&str> =
        REQUIRED_TABLES.iter().copied().filter(|t| !tables.contains(*t)).collect();
    let _ = conn.close().await;

    if missing.is_empty() {
        Ok(())
    } else {
        Err(SarcaError::InvalidBackupFile(format!(
            "backup is missing Sarca tables ({})",
            missing.join(", ")
        )))
    }
}

/// Snapshot the current database next to the archives, and prune old copies.
/// Best-effort: a restore is not worth blocking on the safety net failing, but
/// the operator does need to know it is not there.
async fn write_safety_copy(db: &SqlitePool, dir: &Path) -> Option<String> {
    let target = dir.join(format!("pre-restore-{}.sqlite", timestamp()));
    match snapshot_into(db, &target).await {
        Ok(()) => {
            prune_safety_copies(dir).await;
            tracing::info!("pre-restore safety copy written to {}", target.display());
            Some(target.to_string_lossy().into_owned())
        },
        Err(e) => {
            tracing::warn!("could not write a pre-restore safety copy: {e}");
            None
        },
    }
}

/// Prefixes of the working files a backup or restore creates and removes again.
/// A crash mid-run is the one path that leaves them behind, so startup sweeps
/// them; `pre-restore-` is deliberately not here — those are the safety net.
const SCRATCH_PREFIXES: [&str; 4] = ["snapshot-", "archive-", "staged-", "upload-"];

/// Remove working files a crashed backup or restore left in `WORK_DIR/backups`.
/// Returns how many were removed. Called at startup — see `main`.
pub async fn cleanup_scratch(work_dir: &str) -> usize {
    let dir = Path::new(work_dir).join("backups");
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return 0;
    };

    let mut removed = 0usize;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !SCRATCH_PREFIXES.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        match tokio::fs::remove_file(entry.path()).await {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!("failed to remove {}: {e}", entry.path().display()),
        }
    }
    removed
}

async fn prune_safety_copies(dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    let mut copies: Vec<PathBuf> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("pre-restore-") && name.ends_with(".sqlite") {
            copies.push(entry.path());
        }
    }
    // Names are UTC timestamps in a fixed width, so lexical order is chronological.
    copies.sort();
    let drop_count = copies.len().saturating_sub(SAFETY_COPIES_KEPT);
    for path in copies.into_iter().take(drop_count) {
        if let Err(e) = tokio::fs::remove_file(&path).await {
            tracing::warn!("failed to prune {}: {e}", path.display());
        }
    }
}

/// Copy every table of `snapshot` over the live database, in one transaction.
async fn apply_snapshot(sqlite_path: &str, snapshot: &Path) -> SarcaResult<RestoreOutcome> {
    // Foreign keys off for the duration: the copy order across a dozen
    // interlinked tables is otherwise unsatisfiable, and the snapshot was
    // already consistent when it was taken.
    let mut conn = SqliteConnectOptions::new()
        .filename(sqlite_path)
        .create_if_missing(false)
        .foreign_keys(false)
        .busy_timeout(RESTORE_BUSY_TIMEOUT)
        .connect()
        .await
        .map_err(|e| internal("opening database for restore", &e))?;

    sqlx::query("ATTACH DATABASE ? AS restore_src")
        .bind(snapshot.to_string_lossy().into_owned())
        .execute(&mut conn)
        .await
        .map_err(|e| internal("attaching backup", &e))?;

    let outcome = copy_tables(&mut conn).await;

    let _ = sqlx::query("DETACH DATABASE restore_src").execute(&mut conn).await;
    if outcome.is_ok() {
        // The copy rewrites most of the database; without this the WAL keeps
        // the pre-restore pages around until the next automatic checkpoint.
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&mut conn).await;
    }
    let _ = conn.close().await;

    outcome
}

async fn copy_tables(conn: &mut SqliteConnection) -> SarcaResult<RestoreOutcome> {
    let live = table_names(&mut *conn, "main").await?;
    let incoming = table_names(&mut *conn, "restore_src").await?;

    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *conn)
        .await
        .map_err(|e| internal("starting restore transaction (is another write in flight?)", &e))?;

    match copy_tables_in_transaction(conn, &live, &incoming).await {
        Ok(outcome) => {
            sqlx::query("COMMIT")
                .execute(&mut *conn)
                .await
                .map_err(|e| internal("committing restore", &e))?;
            Ok(outcome)
        },
        Err(e) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            Err(e)
        },
    }
}

async fn copy_tables_in_transaction(
    conn: &mut SqliteConnection,
    live: &HashSet<String>,
    incoming: &HashSet<String>,
) -> SarcaResult<RestoreOutcome> {
    // Triggers describe what a *user edit* means, and a restore is not one.
    // Left in place, `files_sync_event_insert` minted a fresh sync event for
    // every row copied into `files` and `files_sync_event_delete` did the same
    // while the table was being cleared, so the archive's own
    // `file_sync_events` rows then collided on their primary key — a restore
    // that failed or not depending on which table the copy happened to reach
    // first. DDL is transactional here, so a rollback puts them back.
    let triggers = take_triggers(conn).await?;

    // Every live table is emptied, including ones the archive predates: leaving
    // today's rows behind would mix two instances into one database.
    for table in live {
        sqlx::query(&format!("DELETE FROM main.{}", quote_ident(table)))
            .execute(&mut *conn)
            .await
            .map_err(|e| internal(&format!("clearing {table}"), &e))?;
    }

    let mut tables = 0usize;
    let mut rows = 0u64;
    for table in incoming {
        if !live.contains(table) {
            continue;
        }

        let live_columns = column_names(conn, "main", table).await?;
        let incoming_columns = column_names(conn, "restore_src", table).await?;
        let shared: Vec<String> =
            live_columns.into_iter().filter(|c| incoming_columns.contains(c)).collect();
        if shared.is_empty() {
            continue;
        }

        let columns = shared.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
        let quoted = quote_ident(table);
        let affected = sqlx::query(&format!(
            "INSERT INTO main.{quoted} ({columns}) SELECT {columns} FROM restore_src.{quoted}"
        ))
        .execute(&mut *conn)
        .await
        .map_err(|e| internal(&format!("restoring {table}"), &e))?
        .rows_affected();

        tables += 1;
        rows += affected;
    }

    restore_triggers(conn, &triggers).await?;

    let mut skipped_tables: Vec<String> =
        incoming.iter().filter(|t| !live.contains(*t)).cloned().collect();
    skipped_tables.sort();
    if !skipped_tables.is_empty() {
        tracing::warn!(
            "backup carried tables this build does not have: {}",
            skipped_tables.join(", ")
        );
    }

    Ok(RestoreOutcome {
        tables,
        rows,
        skipped_tables,
    })
}

/// Drop every trigger on `main`, returning the `CREATE TRIGGER` statements that
/// [`restore_triggers`] puts back once the copy is done.
async fn take_triggers(conn: &mut SqliteConnection) -> SarcaResult<Vec<String>> {
    let triggers: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, sql FROM main.sqlite_master WHERE type = 'trigger' AND sql IS NOT NULL",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|e| internal("reading triggers", &e))?;

    for (name, _) in &triggers {
        sqlx::query(&format!("DROP TRIGGER main.{}", quote_ident(name)))
            .execute(&mut *conn)
            .await
            .map_err(|e| internal(&format!("dropping trigger {name}"), &e))?;
    }

    Ok(triggers.into_iter().map(|(_, sql)| sql).collect())
}

async fn restore_triggers(conn: &mut SqliteConnection, triggers: &[String]) -> SarcaResult<()> {
    for sql in triggers {
        sqlx::query(sql)
            .execute(&mut *conn)
            .await
            .map_err(|e| internal("recreating a trigger", &e))?;
    }
    Ok(())
}

/// User tables in `schema`, excluding SQLite's own bookkeeping.
async fn table_names<'c, E>(executor: E, schema: &str) -> SarcaResult<HashSet<String>>
where
    E: sqlx::Executor<'c, Database = sqlx::Sqlite>,
{
    let sql = format!(
        "SELECT name FROM {}.sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        quote_ident(schema)
    );
    let names: Vec<String> = sqlx::query_scalar(&sql).fetch_all(executor).await.map_err(|e| {
        SarcaError::InvalidBackupFile(format!("could not read the table list ({e})"))
    })?;
    Ok(names.into_iter().collect())
}

async fn column_names(
    conn: &mut SqliteConnection,
    schema: &str,
    table: &str,
) -> SarcaResult<Vec<String>> {
    sqlx::query_scalar("SELECT name FROM pragma_table_info(?, ?)")
        .bind(table.to_owned())
        .bind(schema.to_owned())
        .fetch_all(conn)
        .await
        .map_err(|e| internal(&format!("reading columns of {schema}.{table}"), &e))
}

/// SQL identifier quoting. The names come from `sqlite_master` of a file the
/// user supplied, so they are never interpolated raw.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sqlx::Executor;

    use super::*;
    use crate::common::db::pool::get_pool;

    async fn pool(path: &Path) -> SqlitePool {
        get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap()
    }

    #[test]
    fn identifiers_are_quoted_not_interpolated() {
        assert_eq!(quote_ident("files"), "\"files\"");
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
    }

    #[tokio::test]
    async fn snapshot_is_a_readable_copy() {
        let dir = tempfile::tempdir().unwrap();
        let db = pool(&dir.path().join("live.sqlite")).await;
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)").await.unwrap();
        db.execute("INSERT INTO t (v) VALUES ('one'), ('two')").await.unwrap();

        let target = dir.path().join("snap.sqlite");
        snapshot_into(&db, &target).await.unwrap();

        let copy = pool(&target).await;
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM t").fetch_one(&copy).await.unwrap();
        assert_eq!(count, 2);
        copy.close().await;
        db.close().await;
    }

    // A crashed run leaves the target behind, and `VACUUM INTO` refuses to
    // overwrite: without the pre-clear every later backup failed.
    #[tokio::test]
    async fn snapshot_overwrites_a_leftover_target() {
        let dir = tempfile::tempdir().unwrap();
        let db = pool(&dir.path().join("live.sqlite")).await;
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").await.unwrap();

        let target = dir.path().join("snap.sqlite");
        tokio::fs::write(&target, b"leftover").await.unwrap();
        snapshot_into(&db, &target).await.unwrap();
        db.close().await;
    }

    #[tokio::test]
    async fn restore_replaces_rows_and_matches_columns_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let live_path = dir.path().join("live.sqlite");
        let live = pool(&live_path).await;
        // Live schema has a column the archive predates.
        live.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT, added TEXT DEFAULT 'new')")
            .await
            .unwrap();
        live.execute("INSERT INTO t (id, v) VALUES (1, 'stale')").await.unwrap();

        let snapshot_path = dir.path().join("snapshot.sqlite");
        let snapshot = pool(&snapshot_path).await;
        snapshot.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)").await.unwrap();
        snapshot
            .execute("INSERT INTO t (id, v) VALUES (7, 'restored'), (8, 'also')")
            .await
            .unwrap();
        snapshot.close().await;

        let outcome = apply_snapshot(live_path.to_str().unwrap(), &snapshot_path).await.unwrap();
        assert_eq!(outcome.rows, 2);
        assert!(outcome.skipped_tables.is_empty());

        let rows: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT id, v, added FROM t ORDER BY id")
                .fetch_all(&live)
                .await
                .unwrap();
        assert_eq!(rows.len(), 2, "the pre-restore row must be gone");
        assert_eq!(rows[0].0, 7);
        assert_eq!(rows[0].2, "new", "a column the backup lacks falls back to its default");
        live.close().await;
    }

    // A table the live schema no longer has must not silently keep its old rows
    // next to the restored ones.
    #[tokio::test]
    async fn restore_empties_tables_the_backup_does_not_carry() {
        let dir = tempfile::tempdir().unwrap();
        let live_path = dir.path().join("live.sqlite");
        let live = pool(&live_path).await;
        live.execute("CREATE TABLE kept (id INTEGER PRIMARY KEY)").await.unwrap();
        live.execute("CREATE TABLE orphan (id INTEGER PRIMARY KEY)").await.unwrap();
        live.execute("INSERT INTO orphan (id) VALUES (1)").await.unwrap();

        let snapshot_path = dir.path().join("snapshot.sqlite");
        let snapshot = pool(&snapshot_path).await;
        snapshot.execute("CREATE TABLE kept (id INTEGER PRIMARY KEY)").await.unwrap();
        snapshot.close().await;

        apply_snapshot(live_path.to_str().unwrap(), &snapshot_path).await.unwrap();

        let left: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM orphan").fetch_one(&live).await.unwrap();
        assert_eq!(left, 0);
        live.close().await;
    }

    #[tokio::test]
    async fn restore_reports_tables_this_build_cannot_hold() {
        let dir = tempfile::tempdir().unwrap();
        let live_path = dir.path().join("live.sqlite");
        let live = pool(&live_path).await;
        live.execute("CREATE TABLE shared (id INTEGER PRIMARY KEY)").await.unwrap();

        let snapshot_path = dir.path().join("snapshot.sqlite");
        let snapshot = pool(&snapshot_path).await;
        snapshot.execute("CREATE TABLE shared (id INTEGER PRIMARY KEY)").await.unwrap();
        snapshot.execute("CREATE TABLE from_the_future (id INTEGER PRIMARY KEY)").await.unwrap();
        snapshot.close().await;

        let outcome = apply_snapshot(live_path.to_str().unwrap(), &snapshot_path).await.unwrap();
        assert_eq!(outcome.skipped_tables, vec!["from_the_future".to_owned()]);
        live.close().await;
    }

    #[tokio::test]
    async fn a_foreign_sqlite_file_is_refused_before_anything_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let stranger_path = dir.path().join("stranger.sqlite");
        let stranger = pool(&stranger_path).await;
        stranger.execute("CREATE TABLE notes (id INTEGER PRIMARY KEY)").await.unwrap();
        stranger.close().await;

        let err = validate_snapshot(&stranger_path).await.unwrap_err();
        assert!(matches!(err, SarcaError::InvalidBackupFile(_)), "{err}");
    }

    #[tokio::test]
    async fn a_file_that_is_not_a_database_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.sqlite");
        tokio::fs::write(&path, b"definitely not sqlite").await.unwrap();
        assert!(validate_snapshot(&path).await.is_err());
    }

    // The end the feature exists for: take a backup here, carry the single file
    // to a fresh install, and find the same storages and files on the other side.
    #[tokio::test]
    async fn a_backup_carries_a_whole_instance_to_another_database() {
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path().join("work");
        tokio::fs::create_dir_all(&work_dir).await.unwrap();

        let source_path = dir.path().join("source.sqlite");
        let source = pool(&source_path).await;
        crate::startup::init_db(&source).await;
        let storage_id = Uuid::new_v4();
        sqlx::query("INSERT INTO storages (id, name, primary_position) VALUES (?, 'Rocks', 1)")
            .bind(storage_id)
            .execute(&source)
            .await
            .unwrap();

        let artifact = BackupService::create(
            &source,
            work_dir.to_str().unwrap(),
            Some("a good long password"),
        )
        .await
        .unwrap();
        assert!(artifact.filename.ends_with(".sarcabak"));
        assert!(artifact.size_bytes > 0);
        source.close().await;

        // A different machine: empty database, current schema, nothing in it.
        let target_path = dir.path().join("target.sqlite");
        let target = pool(&target_path).await;
        crate::startup::init_db(&target).await;

        let staged = dir.path().join("staged.sqlite");
        decode_archive(artifact.file.path(), &staged, Some("a good long password")).await.unwrap();
        validate_snapshot(&staged).await.unwrap();
        apply_snapshot(target_path.to_str().unwrap(), &staged).await.unwrap();

        let name: String = sqlx::query_scalar("SELECT name FROM storages WHERE id = ?")
            .bind(storage_id)
            .fetch_one(&target)
            .await
            .unwrap();
        assert_eq!(name, "Rocks");
        target.close().await;
    }

    #[tokio::test]
    async fn restoring_with_the_wrong_password_asks_again_instead_of_failing_obscurely() {
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path().join("work");
        tokio::fs::create_dir_all(&work_dir).await.unwrap();

        let source = pool(&dir.path().join("source.sqlite")).await;
        crate::startup::init_db(&source).await;
        let artifact = BackupService::create(&source, work_dir.to_str().unwrap(), Some("right"))
            .await
            .unwrap();
        source.close().await;

        let staged = dir.path().join("staged.sqlite");
        let err = decode_archive(artifact.file.path(), &staged, Some("wrong")).await.unwrap_err();
        assert!(matches!(err, SarcaError::InvalidBackupFile(_)), "{err}");

        let err = decode_archive(artifact.file.path(), &staged, None).await.unwrap_err();
        assert!(matches!(err, SarcaError::BackupPasswordRequired), "{err}");
    }

    // Regression: `files_sync_event_insert` fired on every row copied into
    // `files`, so the archive's own `file_sync_events` rows then collided on
    // their primary key. Whether a restore blew up depended on the order the
    // copy happened to visit the two tables in.
    #[tokio::test]
    async fn triggers_do_not_fire_while_the_snapshot_is_copied_in() {
        let dir = tempfile::tempdir().unwrap();

        let schema = [
            "CREATE TABLE files (id INTEGER PRIMARY KEY, path TEXT)",
            "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT)",
            "CREATE TRIGGER files_event_insert AFTER INSERT ON files
             FOR EACH ROW BEGIN
               INSERT INTO events (path) VALUES (NEW.path);
             END",
        ];

        let live_path = dir.path().join("live.sqlite");
        let live = pool(&live_path).await;
        let snapshot_path = dir.path().join("snapshot.sqlite");
        let snapshot = pool(&snapshot_path).await;
        for statement in schema {
            live.execute(statement).await.unwrap();
            snapshot.execute(statement).await.unwrap();
        }

        // The snapshot already carries the event its own insert generated.
        snapshot.execute("INSERT INTO files (id, path) VALUES (1, 'a.txt')").await.unwrap();
        let carried: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events").fetch_one(&snapshot).await.unwrap();
        assert_eq!(carried, 1, "the fixture's own trigger should have run once");
        snapshot.close().await;

        apply_snapshot(live_path.to_str().unwrap(), &snapshot_path).await.unwrap();

        let events: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events").fetch_one(&live).await.unwrap();
        assert_eq!(events, 1, "the copy must not mint a second event");

        // And the trigger is back for ordinary writes afterwards.
        live.execute("INSERT INTO files (id, path) VALUES (2, 'b.txt')").await.unwrap();
        let events: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events").fetch_one(&live).await.unwrap();
        assert_eq!(events, 2);
        live.close().await;
    }

    // A crash mid-backup leaves a whole copy of the database behind. Startup
    // sweeps those, and must not take the safety copies with them.
    #[tokio::test]
    async fn startup_sweeps_working_files_but_spares_safety_copies() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join("backups");
        tokio::fs::create_dir_all(&backups).await.unwrap();
        for name in [
            "snapshot-abc.sqlite",
            "archive-abc.sarcabak",
            "staged-abc.sqlite",
            "upload-abc.sarcabak",
            "pre-restore-20260101-000000.sqlite",
        ] {
            tokio::fs::write(backups.join(name), b"x").await.unwrap();
        }

        assert_eq!(cleanup_scratch(dir.path().to_str().unwrap()).await, 4);
        assert!(backups.join("pre-restore-20260101-000000.sqlite").exists());
    }

    #[tokio::test]
    async fn sweeping_a_work_dir_without_backups_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(cleanup_scratch(dir.path().to_str().unwrap()).await, 0);
    }

    #[tokio::test]
    async fn only_the_newest_safety_copies_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        for stamp in ["20260101-000000", "20260102-000000", "20260103-000000", "20260104-000000"] {
            tokio::fs::write(dir.path().join(format!("pre-restore-{stamp}.sqlite")), b"x")
                .await
                .unwrap();
        }
        tokio::fs::write(dir.path().join("archive-keepme.sarcabak"), b"x").await.unwrap();

        prune_safety_copies(dir.path()).await;

        let mut names = Vec::new();
        let mut entries = tokio::fs::read_dir(dir.path()).await.unwrap();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        assert_eq!(
            names,
            vec![
                "archive-keepme.sarcabak".to_owned(),
                "pre-restore-20260102-000000.sqlite".to_owned(),
                "pre-restore-20260103-000000.sqlite".to_owned(),
                "pre-restore-20260104-000000.sqlite".to_owned(),
            ]
        );
    }
}
