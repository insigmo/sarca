use uuid::Uuid;

use crate::{
    common::{
        access::check_access,
        jwt_manager::AuthUser,
    },
    errors::SarcaResult,
    models::access::AccessType,
    repositories::{access::AccessRepository, sync::SyncRepository},
    schemas::sync::{ChangelogQuery, ChangelogResponse, SnapshotResponse},
};
use sqlx::PgPool;

pub struct SyncService<'d> {
    db: &'d PgPool,
}

impl<'d> SyncService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            db,
        }
    }

    pub async fn changelog(
        &self,
        storage_id: Uuid,
        user: &AuthUser,
        query: ChangelogQuery,
    ) -> SarcaResult<ChangelogResponse> {
        check_access(&AccessRepository::new(self.db), user.id, storage_id, &AccessType::R).await?;

        let cursor = query.cursor.unwrap_or(0).max(0);
        let limit = query.limit.unwrap_or(500).clamp(1, 2000);
        let events = SyncRepository::new(self.db).changelog(storage_id, cursor, limit).await?;
        Ok(ChangelogResponse::new(events, limit))
    }

    pub async fn snapshot(&self, storage_id: Uuid, user: &AuthUser) -> SarcaResult<SnapshotResponse> {
        check_access(&AccessRepository::new(self.db), user.id, storage_id, &AccessType::R).await?;

        let repo = SyncRepository::new(self.db);
        let files = repo.snapshot(storage_id).await?;
        let cursor = repo.max_cursor(storage_id).await?;
        Ok(SnapshotResponse {
            files,
            cursor,
        })
    }
}
