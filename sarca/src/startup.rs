use std::path::Path;

use sqlx::SqlitePool;

use crate::{
    common::password_manager::PasswordManager,
    config::Config,
    errors::SarcaError,
    models::{access::AccessType, users::InDBUser},
    repositories::{
        access::AccessRepository,
        storage_workers::StorageWorkersRepository,
        storages::StoragesRepository,
        users::UsersRepository,
    },
};

/// Current embedded schema version for fresh `SQLite` databases (`schema_version`).
pub const SCHEMA_VERSION: i64 = 1;

#[inline]
#[allow(clippy::too_many_lines)]
pub async fn init_db(db: &SqlitePool) {
    tracing::debug!("initing database");

    let mut transaction = db.begin().await.unwrap();

    for statement in [
        "
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL
        );
        ",
        "
        CREATE TABLE IF NOT EXISTS users (
            id                 BLOB PRIMARY KEY NOT NULL,
            email              TEXT NOT NULL UNIQUE,
            password_hash      TEXT,
            email_verified_at  TEXT,
            -- Unix seconds. Tokens issued at or before this instant are refused,
            -- which is what makes password reset and logout evict live sessions.
            sessions_valid_after INTEGER NOT NULL DEFAULT 0,
            -- Set means the account is disabled: login refused, sessions revoked.
            disabled_at        DATETIME
        );
        ",
        "
        CREATE TABLE IF NOT EXISTS storages (
            id               BLOB PRIMARY KEY NOT NULL,
            name             TEXT NOT NULL,
            primary_position INTEGER NOT NULL DEFAULT 1
        );
        ",
        "
        CREATE TABLE IF NOT EXISTS storage_workers (
            id         BLOB PRIMARY KEY NOT NULL,
            name       TEXT NOT NULL,
            token      TEXT NOT NULL UNIQUE,
            user_id    BLOB NOT NULL REFERENCES users(id)
                                    ON DELETE CASCADE
                                    ON UPDATE CASCADE,
            storage_id BLOB REFERENCES storages(id)
        );
        ",
        "
        CREATE TABLE IF NOT EXISTS access (
            id          BLOB PRIMARY KEY NOT NULL,
            user_id     BLOB NOT NULL REFERENCES users(id)
                                    ON DELETE CASCADE
                                    ON UPDATE CASCADE,
            storage_id  BLOB NOT NULL REFERENCES storages(id)
                                    ON DELETE CASCADE
                                    ON UPDATE CASCADE,
            access_type TEXT NOT NULL CHECK (access_type IN ('r', 'w', 'a')),
            UNIQUE(user_id, storage_id)
        );
        ",
        "
        CREATE TABLE IF NOT EXISTS files (
            id                      BLOB PRIMARY KEY NOT NULL,
            path                    TEXT NOT NULL,
            size                    INTEGER NOT NULL,
            storage_id              BLOB NOT NULL REFERENCES storages(id)
                                                    ON DELETE CASCADE
                                                    ON UPDATE CASCADE,
            is_uploaded             INTEGER NOT NULL,
            thumb_telegram_file_id  TEXT,
            chunk_size_bytes        INTEGER,
            deleted_at              TEXT,
            thumb_telegram_message_id INTEGER,
            created_at              TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at              TEXT NOT NULL DEFAULT (datetime('now')),
            source_created_at       TEXT,
            source_mtime            TEXT,
            content_hash            TEXT,
            preview_telegram_file_id TEXT,
            preview_telegram_message_id INTEGER
        );
        ",
        "
        CREATE UNIQUE INDEX IF NOT EXISTS files_path_storage_id_alive_uidx
          ON files (path, storage_id)
          WHERE deleted_at IS NULL;
        ",
        "
        CREATE TABLE IF NOT EXISTS app_settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
        "
        INSERT INTO app_settings (key, value)
        VALUES ('trash_retention_days', '30')
        ON CONFLICT (key) DO NOTHING;
        ",
        "
        CREATE TABLE IF NOT EXISTS file_chunks (
            id       BLOB PRIMARY KEY NOT NULL,
            file_id  BLOB NOT NULL REFERENCES files(id)
                                    ON DELETE CASCADE
                                    ON UPDATE CASCADE,
            position INTEGER NOT NULL
        );
        ",
        "
        CREATE TABLE IF NOT EXISTS storage_workers_usages (
            id                 BLOB PRIMARY KEY NOT NULL,
            storage_worker_id  BLOB NOT NULL REFERENCES storage_workers(id)
                                                ON DELETE CASCADE
                                                ON UPDATE CASCADE,
            dt                 TEXT DEFAULT (datetime('now'))
        );
        ",
        "
        CREATE TABLE IF NOT EXISTS storage_channels (
            id         BLOB PRIMARY KEY NOT NULL,
            storage_id BLOB NOT NULL REFERENCES storages(id)
                                    ON DELETE CASCADE
                                    ON UPDATE CASCADE,
            position   INTEGER NOT NULL CHECK (position BETWEEN 1 AND 3),
            chat_id    INTEGER NOT NULL UNIQUE,
            name       TEXT NOT NULL,
            status     TEXT NOT NULL DEFAULT 'active',
            UNIQUE(storage_id, position)
        );
        ",
        "
        CREATE TABLE IF NOT EXISTS chunk_replicas (
            id                  BLOB PRIMARY KEY NOT NULL,
            chunk_id            BLOB NOT NULL REFERENCES file_chunks(id)
                                                    ON DELETE CASCADE
                                                    ON UPDATE CASCADE,
            channel_id          BLOB NOT NULL REFERENCES storage_channels(id)
                                                    ON DELETE CASCADE
                                                    ON UPDATE CASCADE,
            telegram_file_id    TEXT,
            telegram_message_id INTEGER,
            status              TEXT NOT NULL DEFAULT 'pending',
            UNIQUE(chunk_id, channel_id)
        );
        ",
        "
        CREATE TABLE IF NOT EXISTS favorites (
            user_id    BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            storage_id BLOB NOT NULL REFERENCES storages(id) ON DELETE CASCADE,
            file_id    BLOB NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (user_id, file_id)
        );
        ",
        "
        CREATE INDEX IF NOT EXISTS favorites_user_storage_idx
          ON favorites (user_id, storage_id);
        ",
        "
        CREATE TABLE IF NOT EXISTS recent_files (
            user_id     BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            storage_id  BLOB NOT NULL REFERENCES storages(id) ON DELETE CASCADE,
            file_id     BLOB NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            viewed_at   TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (user_id, file_id)
        );
        ",
        "
        CREATE INDEX IF NOT EXISTS recent_user_storage_viewed_idx
          ON recent_files (user_id, storage_id, viewed_at DESC);
        ",
        "
        CREATE TABLE IF NOT EXISTS share_links (
            id            BLOB PRIMARY KEY NOT NULL,
            token         TEXT NOT NULL UNIQUE,
            storage_id    BLOB NOT NULL REFERENCES storages(id) ON DELETE CASCADE,
            path          TEXT NOT NULL,
            created_by    BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            created_at    TEXT NOT NULL DEFAULT (datetime('now')),
            expires_at    TEXT,
            password_hash TEXT,
            revoked_at    TEXT
        );
        ",
        "
        CREATE INDEX IF NOT EXISTS share_links_storage_path_idx
          ON share_links (storage_id, path);
        ",
        "
        CREATE INDEX IF NOT EXISTS share_links_created_by_idx
          ON share_links (created_by);
        ",
        "
        DROP TABLE IF EXISTS email_tokens;
        ",
        "
        CREATE TABLE IF NOT EXISTS file_sync_events (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            storage_id    BLOB NOT NULL REFERENCES storages(id) ON DELETE CASCADE,
            file_id       BLOB,
            path          TEXT NOT NULL,
            op            TEXT NOT NULL CHECK (op IN ('upsert', 'delete')),
            size          INTEGER,
            is_file       INTEGER NOT NULL DEFAULT 1,
            content_hash  TEXT,
            source_mtime  TEXT,
            created_at    TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
        "
        CREATE INDEX IF NOT EXISTS file_sync_events_storage_id_id_idx
          ON file_sync_events (storage_id, id);
        ",
        "
        CREATE TABLE IF NOT EXISTS storage_purge_jobs (
            id           BLOB PRIMARY KEY NOT NULL,
            storage_id   BLOB NOT NULL,
            bot_token    TEXT NOT NULL,
            created_at   TEXT NOT NULL DEFAULT (datetime('now')),
            completed_at TEXT
        );
        ",
        "
        CREATE TABLE IF NOT EXISTS storage_purge_messages (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id       BLOB NOT NULL REFERENCES storage_purge_jobs(id) ON DELETE CASCADE,
            chat_id      INTEGER NOT NULL,
            message_id   INTEGER NOT NULL,
            status       TEXT NOT NULL DEFAULT 'pending'
                         CHECK (status IN ('pending', 'in_progress', 'done', 'failed')),
            attempts     INTEGER NOT NULL DEFAULT 0,
            last_error   TEXT,
            updated_at   TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE (job_id, chat_id, message_id)
        );
        ",
        "
        CREATE INDEX IF NOT EXISTS storage_purge_messages_pending_idx
          ON storage_purge_messages (status, id)
          WHERE status = 'pending';
        ",
        // updated_at touch
        "DROP TRIGGER IF EXISTS files_touch_updated_at;",
        r#"
        CREATE TRIGGER files_touch_updated_at
          AFTER UPDATE ON files
          FOR EACH ROW
          WHEN NEW.updated_at IS OLD.updated_at
        BEGIN
          UPDATE files SET updated_at = datetime('now') WHERE id = NEW.id;
        END;
        "#,
        // sync-event triggers (SQLite: separate INSERT/UPDATE/DELETE)
        "DROP TRIGGER IF EXISTS files_sync_event_insert;",
        "DROP TRIGGER IF EXISTS files_sync_event_update;",
        "DROP TRIGGER IF EXISTS files_sync_event_delete;",
        r#"
        CREATE TRIGGER files_sync_event_insert
          AFTER INSERT ON files
          FOR EACH ROW
          WHEN NEW.deleted_at IS NULL
           AND (NEW.is_uploaded OR substr(NEW.path, -1, 1) = '/')
        BEGIN
          INSERT INTO file_sync_events (
            storage_id, file_id, path, op, size, is_file, content_hash, source_mtime
          ) VALUES (
            NEW.storage_id, NEW.id, NEW.path, 'upsert', NEW.size,
            CASE WHEN substr(NEW.path, -1, 1) = '/' THEN 0 ELSE 1 END,
            NEW.content_hash, NEW.source_mtime
          );
        END;
        "#,
        r#"
        CREATE TRIGGER files_sync_event_delete
          AFTER DELETE ON files
          FOR EACH ROW
          WHEN EXISTS (SELECT 1 FROM storages WHERE id = OLD.storage_id)
        BEGIN
          INSERT INTO file_sync_events (
            storage_id, file_id, path, op, size, is_file, content_hash, source_mtime
          ) VALUES (
            OLD.storage_id, OLD.id, OLD.path, 'delete', OLD.size,
            CASE WHEN substr(OLD.path, -1, 1) = '/' THEN 0 ELSE 1 END,
            OLD.content_hash, OLD.source_mtime
          );
        END;
        "#,
        r#"
        CREATE TRIGGER files_sync_event_update
          AFTER UPDATE ON files
          FOR EACH ROW
        BEGIN
          -- path rename: delete old + maybe upsert new
          INSERT INTO file_sync_events (
            storage_id, file_id, path, op, size, is_file, content_hash, source_mtime
          )
          SELECT OLD.storage_id, OLD.id, OLD.path, 'delete', OLD.size,
                 CASE WHEN substr(OLD.path, -1, 1) = '/' THEN 0 ELSE 1 END,
                 OLD.content_hash, OLD.source_mtime
          WHERE OLD.path IS NOT NEW.path;

          INSERT INTO file_sync_events (
            storage_id, file_id, path, op, size, is_file, content_hash, source_mtime
          )
          SELECT NEW.storage_id, NEW.id, NEW.path, 'upsert', NEW.size,
                 CASE WHEN substr(NEW.path, -1, 1) = '/' THEN 0 ELSE 1 END,
                 NEW.content_hash, NEW.source_mtime
          WHERE OLD.path IS NOT NEW.path
            AND NEW.deleted_at IS NULL
            AND (NEW.is_uploaded OR substr(NEW.path, -1, 1) = '/');

          -- soft-delete
          INSERT INTO file_sync_events (
            storage_id, file_id, path, op, size, is_file, content_hash, source_mtime
          )
          SELECT NEW.storage_id, NEW.id, NEW.path, 'delete', NEW.size,
                 CASE WHEN substr(NEW.path, -1, 1) = '/' THEN 0 ELSE 1 END,
                 NEW.content_hash, NEW.source_mtime
          WHERE OLD.path IS NEW.path
            AND NEW.deleted_at IS NOT NULL
            AND OLD.deleted_at IS NULL;

          -- upsert on content/upload/restore changes (same path)
          INSERT INTO file_sync_events (
            storage_id, file_id, path, op, size, is_file, content_hash, source_mtime
          )
          SELECT NEW.storage_id, NEW.id, NEW.path, 'upsert', NEW.size,
                 CASE WHEN substr(NEW.path, -1, 1) = '/' THEN 0 ELSE 1 END,
                 NEW.content_hash, NEW.source_mtime
          WHERE OLD.path IS NEW.path
            AND NEW.deleted_at IS NULL
            AND (NEW.is_uploaded OR substr(NEW.path, -1, 1) = '/')
            AND (
              OLD.deleted_at IS NOT NULL
              OR OLD.is_uploaded IS NOT NEW.is_uploaded
              OR OLD.size IS NOT NEW.size
              OR OLD.content_hash IS NOT NEW.content_hash
              OR OLD.source_mtime IS NOT NEW.source_mtime
            );
        END;
        "#,
    ] {
        sqlx::query(statement)
            .execute(&mut *transaction)
            .await
            .inspect_err(|_e| {
                tracing::error!("error during initing database with query:\n{statement}");
            })
            .unwrap();
    }

    let version_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_version")
        .fetch_one(&mut *transaction)
        .await
        .unwrap();
    if version_count == 0 {
        sqlx::query("INSERT INTO schema_version (version) VALUES (?) ")
            .bind(SCHEMA_VERSION)
            .execute(&mut *transaction)
            .await
            .unwrap();
    }

    transaction.commit().await.unwrap();

    add_missing_columns(db).await;
}

/// Columns added after a table's first release: `CREATE TABLE IF NOT EXISTS` never
/// touches an existing table, so older databases need an explicit `ADD COLUMN`.
/// Duplicate-column errors mean the database is already current.
#[inline]
async fn add_missing_columns(db: &SqlitePool) {
    for (table, column, definition) in [
        ("files", "preview_telegram_file_id", "TEXT"),
        ("files", "preview_telegram_message_id", "INTEGER"),
        ("users", "sessions_valid_after", "INTEGER NOT NULL DEFAULT 0"),
        ("users", "disabled_at", "DATETIME"),
    ] {
        let has_column: bool = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pragma_table_info($1) WHERE name = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_one(db)
        .await
        // On a read error, assume the column exists: never ALTER blindly.
        .map_or(true, |n| n > 0);

        if has_column {
            continue;
        }

        match sqlx::query(&format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"))
            .execute(db)
            .await
        {
            Ok(_) => tracing::info!("migrated: added {table}.{column}"),
            Err(e) => tracing::error!("failed to add {table}.{column}: {e}"),
        }
    }
}

/// Drop stored preview references when the preview encode parameters change.
///
/// The on-disk preview cache is keyed by [`PREVIEW_FORMAT_VERSION`], so it
/// invalidates itself. The preview *documents* on Telegram do not: `preview`
/// prefers `files.preview_telegram_file_id` over re-encoding, so without this
/// every photo uploaded under the old parameters would keep serving the old
/// encode forever. Clearing the reference sends the next open down the
/// re-encode path, which backfills a fresh preview and pays the cost once.
///
/// The marker lives beside the cache in `work_dir`, so a wiped work directory
/// re-runs this — harmless, since a wiped work directory has no cached previews
/// to disagree with either.
///
/// Note: the old preview messages are left in the Telegram channel. Deleting
/// them needs a per-file API round trip and a rate-limit budget; an orphaned
/// document is a few hundred kilobytes.
pub async fn reset_previews_on_format_change(db: &SqlitePool, work_dir: &Path) {
    let marker = work_dir.join("preview_format");
    let current = crate::common::media_cache::PREVIEW_FORMAT_VERSION;

    match tokio::fs::read_to_string(&marker).await {
        Ok(recorded) if recorded.trim() == current => return,
        // A database that predates the marker has previews in the old format,
        // so falling through and clearing them is the correct read of "absent".
        Ok(_) | Err(_) => {},
    }

    match sqlx::query(
        "UPDATE files
            SET preview_telegram_file_id = NULL, preview_telegram_message_id = NULL
          WHERE preview_telegram_file_id IS NOT NULL",
    )
    .execute(db)
    .await
    {
        Ok(result) => {
            let cleared = result.rows_affected();
            if cleared > 0 {
                tracing::info!(
                    "preview format is now {current}: {cleared} preview(s) will rebuild"
                );
            }
        },
        Err(e) => {
            // Leave the marker unwritten so the next boot retries.
            tracing::error!("could not reset previews for format {current}: {e}");
            return;
        },
    }

    if let Err(e) = tokio::fs::create_dir_all(work_dir).await {
        tracing::warn!("could not create {}: {e}", work_dir.display());
        return;
    }
    if let Err(e) = tokio::fs::write(&marker, current).await {
        tracing::warn!("could not record the preview format: {e}");
    }
}

/// Remove storage workers that were never bound to a storage (legacy orphans).
#[inline]
pub async fn delete_orphan_storage_workers(db: &SqlitePool) {
    match StorageWorkersRepository::new(db).delete_orphans().await {
        Ok(0) => {},
        Ok(n) => tracing::info!("deleted {n} orphan storage worker(s) without storage_id"),
        Err(e) => tracing::warn!("orphan storage worker cleanup failed: {e}"),
    }
}

#[inline]
pub async fn create_superuser(db: &SqlitePool, config: &Config) {
    let password_hash = PasswordManager::generate(&config.superuser_pass).unwrap();
    let mut user = InDBUser::new_password(config.superuser_email.clone(), password_hash.clone());
    user.email_verified_at = Some(chrono::Utc::now());
    let result = UsersRepository::new(db).create(user).await;

    match result {
        Ok(_) => tracing::debug!("created superuser"),

        // Keep password in sync with sarca.conf on every boot.
        Err(SarcaError::AlreadyExists(_)) => {
            if let Err(err) = UsersRepository::new(db)
                .update_password_hash(&config.superuser_email, &password_hash)
                .await
            {
                panic!("can't sync superuser password: {err}");
            }
            tracing::debug!("superuser already exists; password synced from config");
        },

        // in case of another error kind -> terminating process
        Err(err) => panic!("can't create superuser; terminating process: {err}"),
    }

    grant_superuser_access_to_all_storages(db, config).await;
}

/// Make sure the superuser holds `A` on every storage.
///
/// Access is per-user rows, and the superuser only ever got one for storages it
/// created itself. A storage created by anyone else was therefore invisible to
/// it: the storage list came back empty, the UI concluded there were no
/// storages and pushed the setup wizard, and the wizard then refused every
/// channel as "already linked to another storage" — a dead end with no way out
/// from inside the app, since granting access itself requires `A` on that very
/// storage.
///
/// Runs on every boot (cheap, idempotent) so a storage created while the
/// superuser was absent is picked up at the next restart.
async fn grant_superuser_access_to_all_storages(db: &SqlitePool, config: &Config) {
    let superuser = match UsersRepository::new(db).get_by_email(&config.superuser_email).await {
        Ok(user) => user,
        Err(e) => {
            tracing::warn!("cannot resolve superuser to grant storage access: {e}");
            return;
        },
    };

    let storage_ids = match StoragesRepository::new(db).list_all_ids().await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("cannot list storages to grant superuser access: {e}");
            return;
        },
    };

    let access_repo = AccessRepository::new(db);
    let mut granted = 0usize;
    for storage_id in storage_ids {
        match access_repo.has_access(superuser.id, storage_id, &AccessType::A).await {
            Ok(true) => continue,
            Ok(false) => {},
            Err(e) => {
                tracing::warn!("access check failed for storage {storage_id}: {e}");
                continue;
            },
        }
        match access_repo.grant_for_user_id(storage_id, superuser.id, AccessType::A).await {
            Ok(()) => granted += 1,
            Err(e) => tracing::warn!("superuser access grant failed for {storage_id}: {e}"),
        }
    }

    if granted > 0 {
        tracing::info!("granted the superuser admin access to {granted} storage(s)");
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uuid::Uuid;

    use super::*;
    use crate::{common::db::pool::get_pool, repositories::users::tests::test_config};

    /// A storage created by someone else used to be invisible to the
    /// superuser, which pushed the UI into the setup wizard and left it stuck
    /// on "channels already linked to another storage".
    #[tokio::test]
    async fn boot_gives_the_superuser_access_to_a_storage_someone_else_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
        init_db(&pool).await;

        let config = test_config("root@sarca.test", "root-pass-123");
        create_superuser(&pool, &config).await;

        // Another account owns the only storage.
        let other = UsersRepository::new(&pool)
            .create(InDBUser::new_password("other@sarca.test".into(), "x".into()))
            .await
            .expect("create other user");
        let storage_id = Uuid::new_v4();
        sqlx::query("INSERT INTO storages (id, name, primary_position) VALUES ($1, 'Cloud', 1)")
            .bind(storage_id)
            .execute(&pool)
            .await
            .unwrap();
        AccessRepository::new(&pool)
            .grant_for_user_id(storage_id, other.id, AccessType::A)
            .await
            .unwrap();

        // Second boot: the superuser must come out of it holding admin access.
        create_superuser(&pool, &config).await;

        let superuser =
            UsersRepository::new(&pool).get_by_email(&config.superuser_email).await.unwrap();
        assert!(
            AccessRepository::new(&pool)
                .has_access(superuser.id, storage_id, &AccessType::A)
                .await
                .unwrap(),
            "the superuser must see every storage, not only its own"
        );
        // The original owner keeps theirs.
        assert!(
            AccessRepository::new(&pool)
                .has_access(other.id, storage_id, &AccessType::A)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn init_db_creates_schema_on_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();
        init_db(&pool).await;
        let v: i64 = sqlx::query_scalar("SELECT version FROM schema_version LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        init_db(&pool).await; // idempotent
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        let tables: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('users','files','chunk_replicas','file_sync_events') ",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tables, 4);
    }

    /// A database created before previews existed keeps its `files` table, so the
    /// new columns can only arrive through `add_missing_columns`.
    #[tokio::test]
    async fn init_db_adds_preview_columns_to_an_older_files_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.sqlite");
        let pool = get_pool(path.to_str().unwrap(), 4, Duration::from_secs(5)).await.unwrap();

        sqlx::query(
            "CREATE TABLE files (
                id BLOB PRIMARY KEY NOT NULL,
                path TEXT NOT NULL,
                size INTEGER NOT NULL,
                storage_id BLOB NOT NULL,
                is_uploaded INTEGER NOT NULL,
                thumb_telegram_file_id TEXT,
                chunk_size_bytes INTEGER,
                deleted_at TEXT,
                thumb_telegram_message_id INTEGER,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                source_created_at TEXT,
                source_mtime TEXT,
                content_hash TEXT
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        init_db(&pool).await;

        for column in ["preview_telegram_file_id", "preview_telegram_message_id"] {
            let present: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('files') WHERE name = $1",
            )
            .bind(column)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(present, 1, "{column} missing after migration");
        }

        init_db(&pool).await; // idempotent: no duplicate-column error
    }

    /// Previews already on Telegram are preferred over re-encoding, so a change
    /// to the encode parameters is invisible until the reference is dropped.
    #[tokio::test]
    async fn a_new_preview_format_clears_stored_previews_once() {
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path().join("work");
        // One connection, so the pragma below applies to every query here.
        let pool =
            get_pool(dir.path().join("db.sqlite").to_str().unwrap(), 1, Duration::from_secs(5))
                .await
                .unwrap();
        init_db(&pool).await;
        // The rows below only need `files.preview_telegram_file_id` to be set;
        // standing up a valid storage and worker graph would test nothing.
        sqlx::query("PRAGMA foreign_keys = OFF").execute(&pool).await.unwrap();

        async fn stored_previews(pool: &SqlitePool) -> i64 {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM files WHERE preview_telegram_file_id IS NOT NULL",
            )
            .fetch_one(pool)
            .await
            .unwrap()
        }
        async fn insert_photo(pool: &SqlitePool, suffix: &str) {
            sqlx::query(
                "INSERT INTO files (id, path, size, storage_id, is_uploaded,
                                    preview_telegram_file_id, preview_telegram_message_id)
                 VALUES (randomblob(16), $1, 1, randomblob(16), 1, $2, 7)",
            )
            .bind(format!("photos/{suffix}.jpg"))
            .bind(format!("tg-{suffix}"))
            .execute(pool)
            .await
            .unwrap();
        }

        insert_photo(&pool, "a").await;
        assert_eq!(stored_previews(&pool).await, 1);

        reset_previews_on_format_change(&pool, &work_dir).await;
        assert_eq!(
            stored_previews(&pool).await,
            0,
            "an unrecorded format must invalidate stored previews"
        );

        // Second boot on the same format: a freshly rebuilt preview survives.
        insert_photo(&pool, "b").await;
        reset_previews_on_format_change(&pool, &work_dir).await;
        assert_eq!(
            stored_previews(&pool).await,
            1,
            "an unchanged format must not keep re-clearing previews"
        );
    }
}
