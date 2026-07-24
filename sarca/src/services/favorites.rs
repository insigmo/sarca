use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::{access::check_access, jwt_manager::AuthUser},
    errors::{SarcaError, SarcaResult},
    models::{access::AccessType, files::FSElement},
    repositories::{
        access::AccessRepository,
        favorites::FavoritesRepository,
        files::FilesRepository,
    },
};

pub struct FavoritesService<'d> {
    favorites: FavoritesRepository<'d>,
    files: FilesRepository<'d>,
    access: AccessRepository<'d>,
}

impl<'d> FavoritesService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            favorites: FavoritesRepository::new(db),
            files: FilesRepository::new(db),
            access: AccessRepository::new(db),
        }
    }

    pub async fn list(&self, storage_id: Uuid, user: &AuthUser) -> SarcaResult<Vec<FSElement>> {
        check_access(&self.access, user.id, storage_id, &AccessType::R).await?;
        self.favorites.list(user.id, storage_id).await
    }

    pub async fn add(&self, storage_id: Uuid, path: &str, user: &AuthUser) -> SarcaResult<()> {
        check_access(&self.access, user.id, storage_id, &AccessType::R).await?;

        let path = normalize_file_path(path)?;
        let file = self.files.get_file_by_path(&path, storage_id).await?;
        if !file.is_uploaded {
            return Err(SarcaError::DoesNotExist("file".to_string()));
        }

        self.favorites.add(user.id, storage_id, file.id).await
    }

    pub async fn remove(&self, storage_id: Uuid, path: &str, user: &AuthUser) -> SarcaResult<()> {
        check_access(&self.access, user.id, storage_id, &AccessType::R).await?;

        let path = normalize_file_path(path)?;
        self.favorites.remove_by_path(user.id, storage_id, &path).await
    }
}

fn normalize_file_path(path: &str) -> SarcaResult<String> {
    let path = path.trim_start_matches('/').to_string();
    if path.is_empty() || path.ends_with('/') || path.starts_with('/') || path.contains("//") {
        return Err(SarcaError::InvalidPath);
    }
    Ok(path)
}
