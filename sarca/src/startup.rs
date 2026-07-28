use std::time::Duration;

use sqlx::PgPool;

use crate::{
    common::{db::pool::get_pool, password_manager::PasswordManager},
    config::Config,
    errors::SarcaError,
    models::users::InDBUser,
    repositories::{storage_workers::StorageWorkersRepository, users::UsersRepository},
};

#[inline]
pub async fn create_db(
    dsn: &str,
    dbname: &str,
    max_connection: u32,
    timeout: Duration,
) -> Result<(), String> {
    let db = get_pool(dsn, max_connection, timeout).await?;

    tracing::debug!("creating database");

    let result = sqlx::query(format!("CREATE DATABASE {dbname}").as_str()).execute(&db).await;

    match &result {
        Ok(_) => {
            tracing::debug!("created database");
            Ok(())
        },
        Err(sqlx::Error::Database(dbe)) => {
            if let Some(code) = dbe.code() {
                if code == "42P04" {
                    tracing::debug!("database already exists; skipping");
                    return Ok(());
                }
            }
            Err(format!("create database failed: {dbe}"))
        },
        Err(e) => Err(format!("create database failed: {e}")),
    }
}

#[inline]
#[allow(clippy::too_many_lines)]
pub async fn init_db(db: &PgPool) {
    tracing::debug!("initing database");

    let mut transaction = db.begin().await.unwrap();

    for statement in [
        "
        CREATE TABLE IF NOT EXISTS users (
            id            UUID         PRIMARY KEY,
            email         VARCHAR(255) NOT NULL UNIQUE,
            password_hash VARCHAR(255) NOT NULL
        );
    ",
        "
        CREATE TABLE IF NOT EXISTS storages (
            id               UUID         PRIMARY KEY,
            name             VARCHAR(255) NOT NULL,
            primary_position SMALLINT     NOT NULL DEFAULT 1
        );

    ",
        "
        CREATE TABLE IF NOT EXISTS storage_workers (
            id         UUID         PRIMARY KEY,
            name       VARCHAR(255) NOT NULL,
            token      VARCHAR(255) NOT NULL UNIQUE,
            user_id    UUID         NOT NULL REFERENCES users
                                            ON DELETE CASCADE 
                                            ON UPDATE CASCADE,
            storage_id UUID         REFERENCES storages
        );

    ",
        "
        DO
        $$
        BEGIN
        IF NOT EXISTS (
            SELECT *
            FROM pg_type typ
            INNER JOIN pg_namespace nsp ON nsp.oid = typ.typnamespace
            WHERE nsp.nspname = current_schema() AND typ.typname = 'access_type'
        ) THEN
            CREATE TYPE access_type AS ENUM ('r', 'w', 'a');
        END IF;
        END;
        $$;
    ",
        "
        CREATE TABLE IF NOT EXISTS access (
            id          UUID        PRIMARY KEY,
            user_id     UUID        NOT NULL REFERENCES users
                                            ON DELETE CASCADE 
                                            ON UPDATE CASCADE,
            storage_id  UUID        NOT NULL REFERENCES storages
                                            ON DELETE CASCADE 
                                            ON UPDATE CASCADE,
            access_type access_type NOT NULL,

            UNIQUE(user_id, storage_id)
        );
    ",
        "
        CREATE TABLE IF NOT EXISTS files (
            id                      UUID         PRIMARY KEY,
            path                    VARCHAR      NOT NULL,
            size                    BigInt       NOT NULL,
            storage_id              UUID         NOT NULL REFERENCES storages
                                                        ON DELETE CASCADE 
                                                        ON UPDATE CASCADE,
            is_uploaded             bool         NOT NULL,
            thumb_telegram_file_id  VARCHAR(255),

            UNIQUE (path, storage_id)
        );
    ",
        "
        ALTER TABLE files
        ADD COLUMN IF NOT EXISTS thumb_telegram_file_id VARCHAR(255);
    ",
        "
        ALTER TABLE files
        ADD COLUMN IF NOT EXISTS chunk_size_bytes BIGINT;
    ",
        "
        ALTER TABLE files
        ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
    ",
        "
        ALTER TABLE files
        ADD COLUMN IF NOT EXISTS thumb_telegram_message_id BIGINT;
    ",
        "
        ALTER TABLE files
        ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
    ",
        "
        ALTER TABLE files
        ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
    ",
        "
        ALTER TABLE files
        ADD COLUMN IF NOT EXISTS source_created_at TIMESTAMPTZ;
    ",
        "
        ALTER TABLE files
        ADD COLUMN IF NOT EXISTS source_mtime TIMESTAMPTZ;
    ",
        r#"
        CREATE OR REPLACE FUNCTION sarca_files_touch_updated_at()
        RETURNS TRIGGER AS $$
        BEGIN
          NEW.updated_at = NOW();
          RETURN NEW;
        END;
        $$ LANGUAGE plpgsql;
    "#,
        "DROP TRIGGER IF EXISTS files_touch_updated_at ON files;",
        r#"
        CREATE TRIGGER files_touch_updated_at
          BEFORE UPDATE ON files
          FOR EACH ROW
          EXECUTE PROCEDURE sarca_files_touch_updated_at();
    "#,
        r#"
        DO $$
        BEGIN
          IF EXISTS (
            SELECT 1 FROM pg_constraint
            WHERE conname = 'files_path_storage_id_key'
          ) THEN
            ALTER TABLE files DROP CONSTRAINT files_path_storage_id_key;
          END IF;
        END $$;
    "#,
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
            id       UUID     PRIMARY KEY,
            file_id  UUID     NOT NULL REFERENCES files 
                                    ON DELETE CASCADE 
                                    ON UPDATE CASCADE,
            position SmallInt NOT NULL
        );
    ",
        "
        CREATE TABLE IF NOT EXISTS storage_workers_usages (
            id                 UUID      PRIMARY KEY,
            storage_worker_id  UUID      NOT NULL REFERENCES storage_workers
                                                ON DELETE CASCADE 
                                                ON UPDATE CASCADE,
            dt                 TIMESTAMP DEFAULT NOW()
        );
    ",
        r#"
        CREATE OR REPLACE FUNCTION public.regexp_quote(IN TEXT)
            RETURNS TEXT
            LANGUAGE plpgsql
            STABLE
        AS $$
            /*******************************************************************************
            * Function Name: regexp_quote
            * In-coming Param:
            *   The string to decoded and convert into a set of text arrays.
            * Returns:
            *   This function produces a TEXT that can be used as a regular expression
            *   pattern that would match the input as if it were a literal pattern.
            * Description:
            *   Takes in a TEXT in and escapes all of the necessary characters so that
            *   the output can be used as a regular expression to match the input as if
            *   it were a literal pattern.
            * Source: https://cwestblog.com/2012/07/10/postgresql-escape-regular-expressions/ * 
            *     The original one doesn't work anymore.
            ******************************************************************************/
        BEGIN
            RETURN REGEXP_REPLACE($1, '([\.\+\*\?\^\$\(\)\[\]\{\}\|\\])', '\\\1', 'g');
        END;
        $$;
    "#,
        // --- multi-chat storage replication ---
        "
        CREATE TABLE IF NOT EXISTS storage_channels (
            id         UUID         PRIMARY KEY,
            storage_id UUID         NOT NULL REFERENCES storages
                                            ON DELETE CASCADE
                                            ON UPDATE CASCADE,
            position   SMALLINT     NOT NULL CHECK (position BETWEEN 1 AND 3),
            chat_id    BigInt       NOT NULL UNIQUE,
            name       VARCHAR(255) NOT NULL,
            status     VARCHAR(16)  NOT NULL DEFAULT 'active',

            UNIQUE(storage_id, position)
        );
    ",
        "
        ALTER TABLE storages
        ADD COLUMN IF NOT EXISTS primary_position SMALLINT NOT NULL DEFAULT 1;
    ",
        // Migrate legacy `storages.chat_id` (1 chat per storage) into a position=1 channel,
        // then drop the column. Idempotent: only runs while the column still exists.
        "
        DO
        $$
        BEGIN
        IF EXISTS (
            SELECT 1
            FROM information_schema.columns
            WHERE table_schema = current_schema()
              AND table_name = 'storages'
              AND column_name = 'chat_id'
        ) THEN
            INSERT INTO storage_channels (id, storage_id, position, chat_id, name, status)
            SELECT gen_random_uuid(), s.id, 1, s.chat_id, s.name, 'active'
            FROM storages s
            WHERE NOT EXISTS (
                SELECT 1 FROM storage_channels sc WHERE sc.storage_id = s.id
            );

            ALTER TABLE storages DROP COLUMN chat_id;
        END IF;
        END;
        $$;
    ",
        "
        CREATE TABLE IF NOT EXISTS chunk_replicas (
            id                  UUID        PRIMARY KEY,
            chunk_id            UUID        NOT NULL REFERENCES file_chunks
                                                    ON DELETE CASCADE
                                                    ON UPDATE CASCADE,
            channel_id          UUID        NOT NULL REFERENCES storage_channels
                                                    ON DELETE CASCADE
                                                    ON UPDATE CASCADE,
            telegram_file_id    VARCHAR(255),
            telegram_message_id BigInt,
            status              VARCHAR(16) NOT NULL DEFAULT 'pending',

            UNIQUE(chunk_id, channel_id)
        );
    ",
        // Migrate legacy `file_chunks.telegram_file_id` into a replica on the storage's
        // primary channel, then drop the column. Idempotent: only runs while it exists.
        "
        DO
        $$
        BEGIN
        IF EXISTS (
            SELECT 1
            FROM information_schema.columns
            WHERE table_schema = current_schema()
              AND table_name = 'file_chunks'
              AND column_name = 'telegram_file_id'
        ) THEN
            INSERT INTO chunk_replicas (id, chunk_id, channel_id, telegram_file_id, \
         telegram_message_id, status)
            SELECT gen_random_uuid(), fc.id, sc.id, fc.telegram_file_id, NULL, 'uploaded'
            FROM file_chunks fc
            JOIN files f ON f.id = fc.file_id
            JOIN storages s ON s.id = f.storage_id
            JOIN storage_channels sc ON sc.storage_id = s.id AND sc.position = s.primary_position
            WHERE NOT EXISTS (
                SELECT 1 FROM chunk_replicas cr WHERE cr.chunk_id = fc.id AND cr.channel_id = sc.id
            );

            ALTER TABLE file_chunks DROP COLUMN telegram_file_id;
        END IF;
        END;
        $$;
    ",
        // --- favorites + recent (Wave 2) ---
        "
        CREATE TABLE IF NOT EXISTS favorites (
            user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            storage_id UUID NOT NULL REFERENCES storages(id) ON DELETE CASCADE,
            file_id    UUID NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (user_id, file_id)
        );
    ",
        "
        CREATE INDEX IF NOT EXISTS favorites_user_storage_idx
          ON favorites (user_id, storage_id);
    ",
        "
        CREATE TABLE IF NOT EXISTS recent_files (
            user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            storage_id  UUID NOT NULL REFERENCES storages(id) ON DELETE CASCADE,
            file_id     UUID NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            viewed_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (user_id, file_id)
        );
    ",
        "
        CREATE INDEX IF NOT EXISTS recent_user_storage_viewed_idx
          ON recent_files (user_id, storage_id, viewed_at DESC);
    ",
        // --- public share links (Wave 3) ---
        "
        CREATE TABLE IF NOT EXISTS share_links (
            id            UUID PRIMARY KEY,
            token         TEXT NOT NULL UNIQUE,
            storage_id    UUID NOT NULL REFERENCES storages(id) ON DELETE CASCADE,
            path          TEXT NOT NULL,
            created_by    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            expires_at    TIMESTAMPTZ NULL,
            password_hash TEXT NULL,
            revoked_at    TIMESTAMPTZ NULL
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
        // --- auth: email verify / password reset ---
        // Add email_verified_at only once; backfill existing users as verified.
        r#"
        DO $$
        BEGIN
          IF NOT EXISTS (
            SELECT 1 FROM information_schema.columns
            WHERE table_schema = current_schema()
              AND table_name = 'users'
              AND column_name = 'email_verified_at'
          ) THEN
            ALTER TABLE users ADD COLUMN email_verified_at TIMESTAMPTZ;
            UPDATE users SET email_verified_at = NOW() WHERE email_verified_at IS NULL;
          END IF;
        END $$;
    "#,
        "
        ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;
    ",
        "
        CREATE TABLE IF NOT EXISTS email_tokens (
            id         UUID PRIMARY KEY,
            user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            purpose    TEXT NOT NULL,
            token_hash TEXT NOT NULL UNIQUE,
            expires_at TIMESTAMPTZ NOT NULL,
            used_at    TIMESTAMPTZ NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );
    ",
        "
        CREATE INDEX IF NOT EXISTS email_tokens_user_purpose_idx
          ON email_tokens (user_id, purpose);
    ",
        "
        DROP TABLE IF EXISTS oauth_accounts;
    ",
        "
        ALTER TABLE files
        ADD COLUMN IF NOT EXISTS content_hash TEXT;
    ",
        "
        CREATE TABLE IF NOT EXISTS file_sync_events (
            id            BIGSERIAL PRIMARY KEY,
            storage_id    UUID NOT NULL REFERENCES storages ON DELETE CASCADE,
            file_id       UUID,
            path          TEXT NOT NULL,
            op            TEXT NOT NULL CHECK (op IN ('upsert', 'delete')),
            size          BIGINT,
            is_file       BOOLEAN NOT NULL DEFAULT TRUE,
            content_hash  TEXT,
            source_mtime  TIMESTAMPTZ,
            created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );
    ",
        "
        CREATE INDEX IF NOT EXISTS file_sync_events_storage_id_id_idx
          ON file_sync_events (storage_id, id);
    ",
        r#"
        CREATE OR REPLACE FUNCTION sarca_files_sync_event()
        RETURNS TRIGGER AS $$
        DECLARE
          is_folder BOOLEAN;
          should_upsert BOOLEAN;
        BEGIN
          IF TG_OP = 'DELETE' THEN
            -- Skip when the parent storage is already gone (ON DELETE CASCADE from
            -- storages). Inserting a sync event would violate file_sync_events_storage_id_fkey
            -- and abort the whole storage delete.
            IF EXISTS (SELECT 1 FROM storages WHERE id = OLD.storage_id) THEN
              INSERT INTO file_sync_events (storage_id, file_id, path, op, size, is_file, content_hash, source_mtime)
              VALUES (
                OLD.storage_id, OLD.id, OLD.path, 'delete', OLD.size,
                RIGHT(OLD.path, 1) <> '/', OLD.content_hash, OLD.source_mtime
              );
            END IF;
            RETURN OLD;
          END IF;

          is_folder := RIGHT(NEW.path, 1) = '/';
          should_upsert := NEW.deleted_at IS NULL AND (NEW.is_uploaded OR is_folder);

          IF TG_OP = 'INSERT' THEN
            IF should_upsert THEN
              INSERT INTO file_sync_events (storage_id, file_id, path, op, size, is_file, content_hash, source_mtime)
              VALUES (
                NEW.storage_id, NEW.id, NEW.path, 'upsert', NEW.size,
                NOT is_folder, NEW.content_hash, NEW.source_mtime
              );
            END IF;
            RETURN NEW;
          END IF;

          -- UPDATE
          IF OLD.path IS DISTINCT FROM NEW.path THEN
            INSERT INTO file_sync_events (storage_id, file_id, path, op, size, is_file, content_hash, source_mtime)
            VALUES (
              OLD.storage_id, OLD.id, OLD.path, 'delete', OLD.size,
              RIGHT(OLD.path, 1) <> '/', OLD.content_hash, OLD.source_mtime
            );
            IF should_upsert THEN
              INSERT INTO file_sync_events (storage_id, file_id, path, op, size, is_file, content_hash, source_mtime)
              VALUES (
                NEW.storage_id, NEW.id, NEW.path, 'upsert', NEW.size,
                NOT is_folder, NEW.content_hash, NEW.source_mtime
              );
            END IF;
            RETURN NEW;
          END IF;

          IF NEW.deleted_at IS NOT NULL AND OLD.deleted_at IS NULL THEN
            INSERT INTO file_sync_events (storage_id, file_id, path, op, size, is_file, content_hash, source_mtime)
            VALUES (
              NEW.storage_id, NEW.id, NEW.path, 'delete', NEW.size,
              NOT is_folder, NEW.content_hash, NEW.source_mtime
            );
            RETURN NEW;
          END IF;

          IF should_upsert AND (
            OLD.deleted_at IS NOT NULL
            OR OLD.is_uploaded IS DISTINCT FROM NEW.is_uploaded
            OR OLD.size IS DISTINCT FROM NEW.size
            OR OLD.content_hash IS DISTINCT FROM NEW.content_hash
            OR OLD.source_mtime IS DISTINCT FROM NEW.source_mtime
          ) THEN
            INSERT INTO file_sync_events (storage_id, file_id, path, op, size, is_file, content_hash, source_mtime)
            VALUES (
              NEW.storage_id, NEW.id, NEW.path, 'upsert', NEW.size,
              NOT is_folder, NEW.content_hash, NEW.source_mtime
            );
          END IF;

          RETURN NEW;
        END;
        $$ LANGUAGE plpgsql;
    "#,
        "DROP TRIGGER IF EXISTS files_sync_event ON files;",
        r#"
        CREATE TRIGGER files_sync_event
          AFTER INSERT OR UPDATE OR DELETE ON files
          FOR EACH ROW
          EXECUTE PROCEDURE sarca_files_sync_event();
    "#,
        // --- durable Telegram purge after storage delete ---
        "
        CREATE TABLE IF NOT EXISTS storage_purge_jobs (
            id           UUID PRIMARY KEY,
            storage_id   UUID NOT NULL,
            bot_token    TEXT NOT NULL,
            created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            completed_at TIMESTAMPTZ NULL
        );
    ",
        "
        CREATE TABLE IF NOT EXISTS storage_purge_messages (
            id           BIGSERIAL PRIMARY KEY,
            job_id       UUID NOT NULL REFERENCES storage_purge_jobs(id) ON DELETE CASCADE,
            chat_id      BIGINT NOT NULL,
            message_id   BIGINT NOT NULL,
            status       TEXT NOT NULL DEFAULT 'pending'
                         CONSTRAINT storage_purge_messages_status_check
                         CHECK (status IN ('pending', 'in_progress', 'done', 'failed')),
            attempts     INT NOT NULL DEFAULT 0,
            last_error   TEXT,
            updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            UNIQUE (job_id, chat_id, message_id)
        );
    ",
        "
        ALTER TABLE storage_purge_messages
          DROP CONSTRAINT IF EXISTS storage_purge_messages_status_check;
    ",
        "
        ALTER TABLE storage_purge_messages
          ADD CONSTRAINT storage_purge_messages_status_check
          CHECK (status IN ('pending', 'in_progress', 'done', 'failed'));
    ",
        "
        CREATE INDEX IF NOT EXISTS storage_purge_messages_pending_idx
          ON storage_purge_messages (status, id)
          WHERE status = 'pending';
    ",
    ] {
        sqlx::query(statement)
            .execute(&mut *transaction)
            .await
            .inspect_err(|_e| {
                tracing::error!("error during initing database with query:\n{statement}");
            })
            .unwrap();
    }

    transaction.commit().await.unwrap();
}

/// Remove storage workers that were never bound to a storage (legacy orphans).
#[inline]
pub async fn delete_orphan_storage_workers(db: &PgPool) {
    match StorageWorkersRepository::new(db).delete_orphans().await {
        Ok(0) => {},
        Ok(n) => tracing::info!("deleted {n} orphan storage worker(s) without storage_id"),
        Err(e) => tracing::warn!("orphan storage worker cleanup failed: {e}"),
    }
}

#[inline]
pub async fn create_superuser(db: &PgPool, config: &Config) {
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
        _ => {
            panic!("can't create superuser; terminating process")
        },
    }
}
