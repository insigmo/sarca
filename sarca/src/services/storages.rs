use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::{access::check_access, jwt_manager::AuthUser},
    errors::{SarcaError, SarcaResult},
    models::{
        access::{AccessType, UserWithAccess},
        storages::{InStorage, Storage, StorageWithInfo},
    },
    repositories::{access::AccessRepository, storages::StoragesRepository},
    schemas::{
        access::{GrantAccess, RestrictAccess},
        storages::{InStorageSchema, UpdateStorageSchema},
    },
};

pub struct StoragesService<'d> {
    repo: StoragesRepository<'d>,
    access_repo: AccessRepository<'d>,
}

impl<'d> StoragesService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        let repo = StoragesRepository::new(db);
        let access_repo = AccessRepository::new(db);
        Self { repo, access_repo }
    }

    pub async fn create(
        &self,
        in_schema: InStorageSchema,
        user: &AuthUser,
    ) -> SarcaResult<Storage> {
        // checking if user already has a storage with such name
        if let Ok(_) = self
            .repo
            .get_by_name_and_user_id(&in_schema.name, user.id)
            .await
        {
            return Err(SarcaError::StorageNameConflict);
        }

        // creating storage
        let in_model = InStorage::new(in_schema.name, in_schema.chat_id);
        let storage = self.repo.create(in_model).await?;
        
        tracing::debug!(
            "[STORAGES SERVICE] Created storage id={}, name={}, chat_id={}",
            storage.id,
            storage.name,
            storage.chat_id
        );

        // setting user as the storage admin
        let access_schema = GrantAccess::new(user.email.clone(), AccessType::A);
        let result = self
            .access_repo
            .create_or_update(storage.id, access_schema)
            .await;
        
        match &result {
            Ok(_) => {
                tracing::debug!(
                    "[STORAGES SERVICE] Successfully granted access to user {} for storage {}",
                    user.email,
                    storage.id
                );
            }
            Err(e) => {
                tracing::error!(
                    "[STORAGES SERVICE] Failed to grant access to user {} for storage {}: {:?}. Rolling back storage creation.",
                    user.email,
                    storage.id,
                    e
                );
                // fallback
                let _ = self.repo.delete_storage(storage.id).await;
            }
        }
        result.map(|_| storage)
    }

    pub async fn list(&self, user: &AuthUser) -> SarcaResult<Vec<StorageWithInfo>> {
        let storages = self.repo.list_by_user_id(user.id).await?;
        tracing::debug!(
            "[STORAGES SERVICE] Listed {} storages for user_id={}",
            storages.len(),
            user.id
        );
        Ok(storages)
    }

    pub async fn get(&self, id: Uuid, user: &AuthUser) -> SarcaResult<Storage> {
        check_access(&self.access_repo, user.id, id, &AccessType::R).await?;

        self.repo.get_by_id(id).await
    }

    pub async fn update(
        &self,
        id: Uuid,
        in_schema: UpdateStorageSchema,
        user: &AuthUser,
    ) -> SarcaResult<Storage> {
        check_access(&self.access_repo, user.id, id, &AccessType::A).await?;

        let name = in_schema.name.trim();
        if let Ok(existing) = self.repo.get_by_name_and_user_id(name, user.id).await {
            if existing.id != id {
                return Err(SarcaError::StorageNameConflict);
            }
            return Ok(existing);
        }

        self.repo.update_name(id, name).await
    }

    pub async fn delete(&self, id: Uuid, user: &AuthUser) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, id, &AccessType::A).await?;

        self.repo.delete_storage(id).await
    }

    pub async fn grant_access(
        &self,
        id: Uuid,
        in_schema: GrantAccess,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, id, &AccessType::A).await?;

        if in_schema.user_email == user.email {
            return Err(SarcaError::CannotManageAccessOfYourself);
        }

        self.access_repo.create_or_update(id, in_schema).await
    }

    pub async fn list_users_with_access(
        &self,
        id: Uuid,
        user: &AuthUser,
    ) -> SarcaResult<Vec<UserWithAccess>> {
        check_access(&self.access_repo, user.id, id, &AccessType::A).await?;

        self.access_repo.list_users_with_access(id).await
    }

    pub async fn restrict_access(
        &self,
        id: Uuid,
        in_schema: RestrictAccess,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, id, &AccessType::A).await?;

        if in_schema.user_id == user.id {
            return Err(SarcaError::CannotManageAccessOfYourself);
        }

        self.access_repo.delete_access(in_schema.user_id, id).await
    }
}
