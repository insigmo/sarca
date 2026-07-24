use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::db::errors::map_not_found,
    errors::{SarcaError, SarcaResult},
    models::access::{AccessType, UserWithAccess},
    schemas::access::GrantAccess,
};

pub const TABLE: &str = "access";

pub struct AccessRepository<'d> {
    db: &'d PgPool,
}

impl<'d> AccessRepository<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            db,
        }
    }

    pub async fn create_or_update(
        &self,
        storage_id: Uuid,
        grant_access: GrantAccess,
    ) -> SarcaResult<()> {
        let id = Uuid::new_v4();

        tracing::debug!(
            "[ACCESS REPO] Attempting to grant access: storage_id={}, user_email={}, \
             access_type={:?}",
            storage_id,
            grant_access.user_email,
            grant_access.access_type
        );

        let result = sqlx::query(
            format!(
                "
                INSERT INTO {TABLE} (id, user_id, storage_id, access_type)
                SELECT $1, u.id, $3, $4
                FROM users u
                WHERE u.email = $2
                ON CONFLICT ON CONSTRAINT access_user_id_storage_id_key
                DO
                    UPDATE SET access_type = $4;
            "
            )
            .as_str(),
        )
        .bind(id)
        .bind(grant_access.user_email.clone())
        .bind(storage_id)
        .bind(grant_access.access_type)
        .execute(self.db)
        .await
        .map_err(|e| {
            match e {
                sqlx::Error::Database(ref dbe) if dbe.is_foreign_key_violation() => {
                    SarcaError::DoesNotExist(format!("storage with id \"{storage_id}\""))
                },
                _ => {
                    tracing::error!("{e}");
                    SarcaError::Unknown
                },
            }
        })?;

        tracing::debug!("[ACCESS REPO] Query affected {} rows", result.rows_affected());

        if result.rows_affected() == 0 {
            tracing::error!(
                "[ACCESS REPO] User with email \"{}\" not found in users table",
                grant_access.user_email
            );
            return Err(SarcaError::DoesNotExist(format!(
                "user with email \"{}\"",
                grant_access.user_email
            )));
        }

        tracing::debug!(
            "[ACCESS REPO] Successfully granted access to user {} for storage {}",
            grant_access.user_email,
            storage_id
        );

        Ok(())
    }

    pub async fn list_users_with_access(
        &self,
        storage_id: Uuid,
    ) -> SarcaResult<Vec<UserWithAccess>> {
        sqlx::query_as(
            format!(
                "
            SELECT u.id AS id, u.email AS email, a.access_type AS access_type
            FROM {TABLE} a
            JOIN users u ON a.user_id = u.id
            WHERE a.storage_id = $1
        "
            )
            .as_str(),
        )
        .bind(storage_id)
        .fetch_all(self.db)
        .await
        .map_err(|e| map_not_found(&e, "user"))
    }

    #[inline]
    pub async fn has_access(
        &self,
        user_id: Uuid,
        storage_id: Uuid,
        access_type: &AccessType,
    ) -> SarcaResult<bool> {
        let access_type_filter = match access_type {
            AccessType::R => "",
            AccessType::W => "AND access_type in ('w', 'a')",
            AccessType::A => "AND access_type = 'a'",
        };

        let has_access: (_,) = sqlx::query_as(
            format!(
                "
            SELECT COUNT(*) > 0
            FROM {TABLE}
            WHERE user_id = $1 AND storage_id = $2 {access_type_filter};
        "
            )
            .as_str(),
        )
        .bind(user_id)
        .bind(storage_id)
        .fetch_one(self.db)
        .await
        .map_err(|e| map_not_found(&e, "access"))?;

        Ok(has_access.0)
    }

    pub async fn delete_access(&self, user_id: Uuid, storage_id: Uuid) -> SarcaResult<()> {
        sqlx::query(
            format!(
                "
            DELETE FROM {TABLE}
            WHERE user_id = $1 AND storage_id = $2
        "
            )
            .as_str(),
        )
        .bind(user_id)
        .bind(storage_id)
        .execute(self.db)
        .await
        .map_err(|e| map_not_found(&e, "access"))?;

        Ok(())
    }
}
