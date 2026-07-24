use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::{access::check_access, jwt_manager::AuthUser},
    errors::{SarcaError, SarcaResult},
    models::{access::AccessType, files::FSElement},
    repositories::{
        access::AccessRepository, favorites::FavoritesRepository, files::FilesRepository,
    },
};

pub struct FavoritesService<'d> {
    favorites_repo: FavoritesRepository<'d>,
    files_repo: FilesRepository<'d>,
    access_repo: AccessRepository<'d>,
}

impl<'d> FavoritesService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            favorites_repo: FavoritesRepository::new(db),
            files_repo: FilesRepository::new(db),
            access_repo: AccessRepository::new(db),
        }
    }

    pub async fn list(
        &self,
        storage_id: Uuid,
        user: &AuthUser,
    ) -> SarcaResult<Vec<FSElement>> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::R).await?;
        self.favorites_repo.list(user.id, storage_id).await
    }

    pub async fn add(
        &self,
        storage_id: Uuid,
        path: &str,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::R).await?;

        let path = normalize_file_path(path)?;
        let file = self.files_repo.get_file_by_path(&path, storage_id).await?;
        if !file.is_uploaded {
            return Err(SarcaError::DoesNotExist("file".to_string()));
        }

        self.favorites_repo
            .add(user.id, storage_id, file.id)
            .await
    }

    pub async fn remove(
        &self,
        storage_id: Uuid,
        path: &str,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::R).await?;

        let path = normalize_file_path(path)?;
        self.favorites_repo
            .remove_by_path(user.id, storage_id, &path)
            .await
    }
}

fn normalize_file_path(path: &str) -> SarcaResult<String> {
    let path = path.trim_start_matches('/').to_string();
    if path.is_empty() || path.ends_with('/') || path.starts_with('/') || path.contains("//") {
        return Err(SarcaError::InvalidPath);
    }
    Ok(path)
}
