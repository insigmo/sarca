use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::{access::check_access, jwt_manager::AuthUser},
    errors::{SarcaError, SarcaResult},
    models::{access::AccessType, files::FSElement},
    repositories::{
        access::AccessRepository,
        files::FilesRepository,
        recent_files::RecentFilesRepository,
    },
};

pub struct RecentService<'d> {
    recent: RecentFilesRepository<'d>,
    files: FilesRepository<'d>,
    access: AccessRepository<'d>,
}

impl<'d> RecentService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            recent: RecentFilesRepository::new(db),
            files: FilesRepository::new(db),
            access: AccessRepository::new(db),
        }
    }

    pub async fn list(&self, storage_id: Uuid, user: &AuthUser) -> SarcaResult<Vec<FSElement>> {
        check_access(&self.access, user.id, storage_id, &AccessType::R).await?;
        self.recent.list(user.id, storage_id).await
    }

    pub async fn record(&self, storage_id: Uuid, path: &str, user: &AuthUser) -> SarcaResult<()> {
        check_access(&self.access, user.id, storage_id, &AccessType::R).await?;

        let path = normalize_file_path(path)?;
        let file = self.files.get_file_by_path(&path, storage_id).await?;
        if !file.is_uploaded {
            return Err(SarcaError::DoesNotExist("file".to_string()));
        }

        self.recent.upsert_and_trim(user.id, storage_id, file.id).await
    }
}

fn normalize_file_path(path: &str) -> SarcaResult<String> {
    let path = path.trim_start_matches('/').to_string();
    if path.is_empty() || path.ends_with('/') || path.starts_with('/') || path.contains("//") {
        return Err(SarcaError::InvalidPath);
    }
    Ok(path)
}
