use sqlx::PgPool;
use std::path::PathBuf;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::{
    common::{
        access::check_access,
        channels::{ClientData, ClientMessage, ClientSender, StorageManagerData, UploadFileData},
        jwt_manager::AuthUser,
    },
    errors::{SarcaError, SarcaResult},
    models::{
        access::AccessType,
        files::{FSElement, File, InFile, SearchFSElement},
    },
    repositories::{
        access::AccessRepository, files::FilesRepository, storage_workers::StorageWorkersRepository,
    },
    schemas::files::{InFileSchema, InFolderSchema},
};

pub struct FilesService<'d> {
    repo: FilesRepository<'d>,
    storage_workers_repo: StorageWorkersRepository<'d>,
    access_repo: AccessRepository<'d>,
    tx: ClientSender,
}

impl<'d> FilesService<'d> {
    pub fn new(db: &'d PgPool, tx: ClientSender) -> Self {
        let repo = FilesRepository::new(db);
        let storage_workers_repo = StorageWorkersRepository::new(db);
        let access_repo = AccessRepository::new(db);
        Self {
            repo,
            access_repo,
            storage_workers_repo,
            tx,
        }
    }

    pub async fn create_folder(
        &self,
        in_schema: InFolderSchema,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        // 0. checking access
        check_access(
            &self.access_repo,
            user.id,
            in_schema.storage_id,
            &AccessType::W,
        )
        .await?;

        // 1. validation
        if !Self::validate_filepath(&in_schema.parent_path) {
            return Err(SarcaError::InvalidPath);
        }
        if in_schema.folder_name.contains(r"/") {
            return Err(SarcaError::InvalidFolderName);
        }

        // 2. constructing final values
        let path = if !in_schema.parent_path.is_empty() {
            format!("{}/{}/", in_schema.parent_path, in_schema.folder_name)
        } else {
            format!("{}/", in_schema.folder_name)
        };
        let in_file = InFile::new(path, 0, in_schema.storage_id);

        // 3. saving to db
        self.repo.create_folder(in_file).await.map(|_| ())
    }

    pub async fn upload_to(&self, in_schema: InFileSchema, user: &AuthUser) -> SarcaResult<()> {
        // 0. checking access
        check_access(
            &self.access_repo,
            user.id,
            in_schema.storage_id,
            &AccessType::W,
        )
        .await?;

        // 1. check whether storage got workers
        Self::check_storage_workers(&self, in_schema.storage_id).await?;

        // 2. path validation
        if !Self::validate_filepath(&in_schema.path) {
            return Err(SarcaError::InvalidPath);
        }

        let in_file = InFile::new(in_schema.path, in_schema.size, in_schema.storage_id);

        // 3. saving file to db
        let file = self.repo.create_file(in_file).await?;

        self._upload_from_path(file, in_schema.file_path, in_schema.size)
            .await
    }

    pub async fn upload_anyway_from_path(
        &self,
        in_file: InFile,
        file_path: PathBuf,
        file_size: i64,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        // 0. checking access
        check_access(
            &self.access_repo,
            user.id,
            in_file.storage_id,
            &AccessType::W,
        )
        .await?;

        // 1. check whether storage got workers
        Self::check_storage_workers(&self, in_file.storage_id).await?;

        // 2. saving file in db
        let file = self.repo.create_file_anyway(in_file).await?;

        self._upload_from_path(file, file_path, file_size).await
    }

    async fn _upload_from_path(
        &self,
        file: File,
        file_path: PathBuf,
        file_size: i64,
    ) -> SarcaResult<()> {
        let (resp_tx, resp_rx) = oneshot::channel();

        let message = ClientMessage {
            data: ClientData::UploadFile(UploadFileData {
                file_id: file.id,
                file_path,
                file_size,
            }),
            tx: resp_tx,
        };

        tracing::debug!("sending task to manager");
        let _ = self.tx.send(message).await;

        // 3. waiting for a storage manager result
        let message_back = match resp_rx.await.unwrap().data {
            StorageManagerData::UploadFile(r) => r,
        };
        if let Err(e) = message_back.and({
            tracing::debug!("file loaded successfully");

            // 4. setting file as uploaded
            self.repo.set_as_uploaded(file.id).await
        }) {
            tracing::error!("{e}");

            // fallback logic: deleting file
            let _ = self.repo.delete_with_folders(file.id).await;

            return Err(e);
        };

        Ok(())
    }

    async fn check_storage_workers(&self, storage_id: Uuid) -> SarcaResult<()> {
        if !self
            .storage_workers_repo
            .storage_has_any(storage_id)
            .await?
        {
            Err(SarcaError::StorageDoesNotHaveWorkers)
        } else {
            Ok(())
        }
    }

    pub async fn list_dir(
        self,
        storage_id: Uuid,
        path: &str,
        user: &AuthUser,
    ) -> SarcaResult<Vec<FSElement>> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::R).await?;

        self.repo.list_dir(storage_id, path).await
    }

    pub async fn search(
        self,
        storage_id: Uuid,
        path: &str,
        search_path: &str,
        user: &AuthUser,
    ) -> SarcaResult<Vec<SearchFSElement>> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::R).await?;

        self.repo.search(search_path, path, storage_id).await
    }

    pub async fn delete(
        &self,
        path: &str,
        storage_id: Uuid,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        // 0. checking access
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;

        // 1. path validation
        if !Self::validate_path(path) {
            return Err(SarcaError::InvalidPath);
        }

        // 2. deleting file
        self.repo.delete(path, storage_id).await
    }

    pub async fn rename(
        &self,
        storage_id: Uuid,
        old_path: &str,
        new_path: &str,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;

        if !Self::validate_path(old_path) || !Self::validate_path(new_path) {
            return Err(SarcaError::InvalidPath);
        }
        if old_path.ends_with('/') != new_path.ends_with('/') {
            return Err(SarcaError::InvalidPath);
        }

        self.repo.update_path(old_path, new_path, storage_id).await
    }

    pub async fn move_to(
        &self,
        storage_id: Uuid,
        path: &str,
        destination_folder: &str,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;

        if !Self::validate_path(path) {
            return Err(SarcaError::InvalidPath);
        }

        let dest = destination_folder.trim_end_matches('/');
        if !dest.is_empty() && !Self::validate_path(dest) {
            return Err(SarcaError::InvalidPath);
        }

        let new_path = Self::path_in_folder(path, dest);
        if !Self::validate_path(&new_path) {
            return Err(SarcaError::InvalidPath);
        }

        self.repo.update_path(path, &new_path, storage_id).await
    }

    /// Build a new path when renaming by basename only.
    pub fn rename_with_new_name(old_path: &str, new_name: &str) -> SarcaResult<String> {
        if new_name.is_empty() || new_name.contains('/') {
            return Err(SarcaError::InvalidFolderName);
        }

        let is_folder = old_path.ends_with('/');
        let trimmed = old_path.trim_end_matches('/');
        let parent = trimmed
            .rsplit_once('/')
            .map(|(p, _)| p)
            .unwrap_or("");

        let new_path = if parent.is_empty() {
            new_name.to_owned()
        } else {
            format!("{parent}/{new_name}")
        };

        Ok(if is_folder {
            format!("{new_path}/")
        } else {
            new_path
        })
    }

    /////////////////////////////////////////////////////////////////////
    ////    Helpers
    /////////////////////////////////////////////////////////////////////

    fn path_in_folder(path: &str, dest_folder: &str) -> String {
        let is_folder = path.ends_with('/');
        let trimmed = path.trim_end_matches('/');
        let name = trimmed.rsplit('/').next().unwrap_or(trimmed);

        let new_path = if dest_folder.is_empty() {
            name.to_owned()
        } else {
            format!("{dest_folder}/{name}")
        };

        if is_folder {
            format!("{new_path}/")
        } else {
            new_path
        }
    }

    fn validate_filepath(path: &str) -> bool {
        Self::validate_path(path) && !path.ends_with(r"/")
    }

    fn validate_path(path: &str) -> bool {
        !path.starts_with(r"/") && !path.contains(r"//")
    }
}
