use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::{access::check_access, jwt_manager::AuthUser},
    errors::{SarcaError, SarcaResult},
    models::{
        access::AccessType,
        storage_workers::{InStorageWorker, StorageWorker},
    },
    repositories::{access::AccessRepository, storage_workers::StorageWorkersRepository},
    schemas::storage_workers::InStorageWorkerSchema,
};

pub struct StorageWorkersService<'d> {
    repo: StorageWorkersRepository<'d>,
    access_repo: AccessRepository<'d>,
}

impl<'d> StorageWorkersService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        let repo = StorageWorkersRepository::new(db);
        let access_repo = AccessRepository::new(db);
        Self {
            repo,
            access_repo,
        }
    }

    pub async fn create(
        &self,
        in_schema: InStorageWorkerSchema,
        user: &AuthUser,
    ) -> SarcaResult<StorageWorker> {
        let storage_id = in_schema.storage_id.ok_or(SarcaError::WorkerRequiresStorage)?;
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;

        // checking if user already has a storage worker with such name
        if self.repo.get_by_name_and_user_id(&in_schema.name, user.id).await.is_ok() {
            return Err(SarcaError::StorageWorkerNameConflict);
        }

        // creating storage worker
        let in_model =
            InStorageWorker::new(in_schema.name, user.id, in_schema.token, Some(storage_id));
        self.repo.create(in_model).await
    }

    pub async fn list(&self, user: &AuthUser) -> SarcaResult<Vec<StorageWorker>> {
        self.repo.list_by_user_id(user.id).await
    }

    pub async fn delete(&self, id: Uuid, user: &AuthUser) -> SarcaResult<()> {
        self.repo.get_by_id_and_user_id(id, user.id).await?;
        self.repo.delete(id, user.id).await
    }

    pub async fn has_storage_workers(
        &self,
        storage_id: Uuid,
        user: &AuthUser,
    ) -> SarcaResult<bool> {
        // 0. checking access
        check_access(&self.access_repo, user.id, storage_id, &AccessType::R).await?;

        self.repo.storage_has_any(storage_id).await
    }
}
