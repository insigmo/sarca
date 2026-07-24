use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::{access::check_access, jwt_manager::AuthUser},
    errors::{SarcaError, SarcaResult},
    models::{access::AccessType, files::FSElement},
    repositories::{
        access::AccessRepository, files::FilesRepository, recent_files::RecentFilesRepository,
    },
};

pub struct RecentService<'d> {
    recent_repo: RecentFilesRepository<'d>,
    files_repo: FilesRepository<'d>,
    access_repo: AccessRepository<'d>,
}

impl<'d> RecentService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            recent_repo: RecentFilesRepository::new(db),
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
        self.recent_repo.list(user.id, storage_id).await
    }

    pub async fn record(
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

        self.recent_repo
            .upsert_and_trim(user.id, storage_id, file.id)
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
