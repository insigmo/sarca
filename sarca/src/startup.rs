use std::time::Duration;

use sqlx::PgPool;

use crate::{
    common::{db::pool::get_pool, jwt_manager::AuthUser, password_manager::PasswordManager},
    config::Config,
    errors::SarcaError,
    models::users::InDBUser,
    repositories::{storages::StoragesRepository, users::UsersRepository},
    schemas::{
        storage_workers::InStorageWorkerSchema,
        storages::InStorageSchema,
    },
    services::{storage_workers::StorageWorkersService, storages::StoragesService},
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

    let result = sqlx::query(format!("CREATE DATABASE {dbname}").as_str())
        .execute(&db)
        .await;

    match &result {
        Ok(_) => {
            tracing::debug!("created database");
            return Ok(());
        }
        Err(sqlx::Error::Database(dbe)) => {
            if let Some(code) = dbe.code() {
                if code == "42P04" {
                    tracing::debug!("database already exists; skipping");
                    return Ok(());
                }
            }
            return Err(format!("create database failed: {dbe}"));
        }
        Err(e) => return Err(format!("create database failed: {e}")),
    }
}

#[inline]
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
            id      UUID         PRIMARY KEY,
            name    VARCHAR(255) NOT NULL,
            chat_id BigInt       NOT NULL UNIQUE
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
        CREATE TABLE IF NOT EXISTS file_chunks (
            id               UUID         PRIMARY KEY,
            file_id          UUID         NOT NULL REFERENCES files 
                                                ON DELETE CASCADE 
                                                ON UPDATE CASCADE,
            telegram_file_id VARCHAR(255) NOT NULL,
            position         SmallInt     NOT NULL
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
    ] {
        sqlx::query(statement)
            .execute(&mut *transaction)
            .await
            .map_err(|e| {
                tracing::error!("error during initing database with query:\n{statement}");
                e
            })
            .unwrap();
    }

    transaction.commit().await.unwrap();
}

#[inline]
pub async fn create_superuser(db: &PgPool, config: &Config) {
    let password_hash = PasswordManager::generate(&config.superuser_pass).unwrap();
    let user = InDBUser::new(config.superuser_email.clone(), password_hash.clone());
    let result = UsersRepository::new(&db).create(user).await;

    match result {
        Ok(_) => tracing::debug!("created superuser"),

        // Keep password in sync with sarca.conf on every boot.
        Err(e) if matches!(e, SarcaError::AlreadyExists(_)) => {
            if let Err(err) = UsersRepository::new(&db)
                .update_password_hash(&config.superuser_email, &password_hash)
                .await
            {
                panic!("can't sync superuser password: {err}");
            }
            tracing::debug!("superuser already exists; password synced from config");
        }

        // in case of another error kind -> terminating process
        _ => {
            panic!("can't create superuser; terminating process")
        }
    };
}

/// Convert a channel id (without `-100`) into a Telegram chat_id for channels/supergroups.
/// Negative values are treated as already-complete chat ids.
pub(crate) fn channel_id_to_chat_id(channel_id: i64) -> i64 {
    if channel_id < 0 {
        return channel_id;
    }
    format!("-100{channel_id}")
        .parse()
        .expect("channel id should form a valid Telegram chat_id")
}

/// If `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHANNEL_ID`, and `STORAGE_NAME` are all set,
/// create the storage and attach the bot for the superuser. Otherwise log that
/// the user should configure them via the UI.
#[inline]
pub async fn bootstrap_storage_from_env(db: &PgPool, config: &Config) {
    let token = config.telegram_bot_token.as_deref();
    let channel_id = config.telegram_channel_id;
    let storage_name = config.storage_name.as_deref();

    let any_set = token.is_some() || channel_id.is_some() || storage_name.is_some();
    let all_set = token.is_some() && channel_id.is_some() && storage_name.is_some();

    if !any_set {
        tracing::info!(
            "TELEGRAM_BOT_TOKEN / TELEGRAM_CHANNEL_ID / STORAGE_NAME not set — \
             create a storage and register a bot via the UI \
             (Storages → New storage, Storage workers → New worker)"
        );
        return;
    }

    if !all_set {
        tracing::warn!(
            "Incomplete env bootstrap: set all of TELEGRAM_BOT_TOKEN, TELEGRAM_CHANNEL_ID, \
             and STORAGE_NAME, or leave all empty and configure via the UI"
        );
        return;
    }

    let token = token.expect("checked above");
    let channel_id = channel_id.expect("checked above");
    let storage_name = storage_name.expect("checked above");
    let chat_id = channel_id_to_chat_id(channel_id);

    let user = match UsersRepository::new(db)
        .get_by_email(&config.superuser_email)
        .await
    {
        Ok(user) => AuthUser::new(user.id, user.email),
        Err(e) => {
            tracing::error!("env bootstrap: cannot load superuser: {e}");
            return;
        }
    };

    let storages = StoragesService::new(db);
    let storage = match storages
        .create(
            InStorageSchema {
                name: storage_name.to_owned(),
                chat_id,
            },
            &user,
        )
        .await
    {
        Ok(storage) => {
            tracing::info!(
                "env bootstrap: created storage \"{}\" (chat_id={})",
                storage.name,
                storage.chat_id
            );
            storage
        }
        Err(SarcaError::StorageNameConflict) => {
            match StoragesRepository::new(db)
                .get_by_name_and_user_id(storage_name, user.id)
                .await
            {
                Ok(storage) => {
                    tracing::debug!(
                        "env bootstrap: storage \"{}\" already exists; reusing",
                        storage_name
                    );
                    storage
                }
                Err(e) => {
                    tracing::error!(
                        "env bootstrap: storage name conflict but lookup failed: {e}"
                    );
                    return;
                }
            }
        }
        Err(SarcaError::StorageChatIdConflict) => {
            tracing::warn!(
                "env bootstrap: chat_id {chat_id} already used by another storage; \
                 configure via the UI or pick a different TELEGRAM_CHANNEL_ID"
            );
            return;
        }
        Err(e) => {
            tracing::error!("env bootstrap: failed to create storage: {e}");
            return;
        }
    };

    let workers = StorageWorkersService::new(db);
    match workers
        .create(
            InStorageWorkerSchema {
                name: storage_name.to_owned(),
                token: token.to_owned(),
                storage_id: Some(storage.id),
            },
            &user,
        )
        .await
    {
        Ok(_) => tracing::info!(
            "env bootstrap: attached bot as storage worker \"{}\"",
            storage_name
        ),
        Err(SarcaError::StorageWorkerNameConflict)
        | Err(SarcaError::StorageWorkerTokenConflict) => {
            tracing::debug!("env bootstrap: storage worker already exists; skipping")
        }
        Err(e) => tracing::error!("env bootstrap: failed to create storage worker: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::channel_id_to_chat_id;

    #[test]
    fn prepends_minus_100_for_positive_channel_id() {
        assert_eq!(channel_id_to_chat_id(1234567890), -1001234567890);
    }

    #[test]
    fn keeps_negative_chat_id() {
        assert_eq!(channel_id_to_chat_id(-1001234567890), -1001234567890);
        assert_eq!(channel_id_to_chat_id(-456), -456);
    }
}
