use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::{
        access::check_access, jwt_manager::AuthUser, password_manager::PasswordManager,
    },
    errors::{SarcaError, SarcaResult},
    models::{access::AccessType, files::FSElement, share_links::ShareLink},
    repositories::{
        access::AccessRepository, files::FilesRepository, share_links::ShareLinksRepository,
    },
    schemas::shares::{PublicShareMetaSchema, ShareLinkSchema},
};

pub struct SharesService<'d> {
    shares_repo: ShareLinksRepository<'d>,
    files_repo: FilesRepository<'d>,
    access_repo: AccessRepository<'d>,
}

impl<'d> SharesService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            shares_repo: ShareLinksRepository::new(db),
            files_repo: FilesRepository::new(db),
            access_repo: AccessRepository::new(db),
        }
    }

    pub async fn create(
        &self,
        storage_id: Uuid,
        path: &str,
        expires_at: Option<DateTime<Utc>>,
        password: Option<&str>,
        user: &AuthUser,
    ) -> SarcaResult<ShareLinkSchema> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;

        let path = self.normalize_and_resolve_target(storage_id, path).await?;

        if let Some(exp) = expires_at {
            if exp <= Utc::now() {
                return Err(SarcaError::InvalidShareExpiry);
            }
        }

        let password_hash = match password {
            Some(p) if !p.is_empty() => Some(PasswordManager::generate(p)?),
            _ => None,
        };

        let id = Uuid::new_v4();
        // 122+ bits from UUID v4; second UUID adds more entropy → 64 hex chars.
        let token = format!(
            "{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );

        let link = self
            .shares_repo
            .create(
                id,
                &token,
                storage_id,
                &path,
                user.id,
                expires_at,
                password_hash.as_deref(),
            )
            .await?;

        Ok(ShareLinkSchema::from_link(&link))
    }

    pub async fn list(
        &self,
        storage_id: Uuid,
        user: &AuthUser,
    ) -> SarcaResult<Vec<ShareLinkSchema>> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;
        let links = self.shares_repo.list_for_storage(storage_id).await?;
        Ok(links.iter().map(ShareLinkSchema::from_link).collect())
    }

    pub async fn revoke(
        &self,
        storage_id: Uuid,
        share_id: Uuid,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::W).await?;
        self.shares_repo.revoke(share_id, storage_id).await
    }

    /// Normalize path and confirm a live uploaded file or folder exists.
    async fn normalize_and_resolve_target(
        &self,
        storage_id: Uuid,
        path: &str,
    ) -> SarcaResult<String> {
        let path = normalize_share_target_path(path)?;

        if path.ends_with('/') {
            match self.files_repo.get_file_by_path(&path, storage_id).await {
                Ok(_) => return Ok(path),
                Err(SarcaError::DoesNotExist(_)) => {
                    let files = self
                        .files_repo
                        .list_uploaded_files_under(storage_id, &path)
                        .await?;
                    if files.is_empty() {
                        return Err(SarcaError::DoesNotExist("folder".to_owned()));
                    }
                    return Ok(path);
                }
                Err(e) => return Err(e),
            }
        }

        match self.files_repo.get_file_by_path(&path, storage_id).await {
            Ok(f) if f.is_uploaded => Ok(path),
            Ok(_) => Err(SarcaError::DoesNotExist("file".to_owned())),
            Err(SarcaError::DoesNotExist(_)) => {
                let folder = format!("{path}/");
                match self.files_repo.get_file_by_path(&folder, storage_id).await {
                    Ok(_) => Ok(folder),
                    Err(SarcaError::DoesNotExist(_)) => {
                        let files = self
                            .files_repo
                            .list_uploaded_files_under(storage_id, &folder)
                            .await?;
                        if files.is_empty() {
                            Err(SarcaError::DoesNotExist("file".to_owned()))
                        } else {
                            Ok(folder)
                        }
                    }
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }
}

/// Public (unauthenticated) share access helpers.
pub struct PublicSharesService<'d> {
    shares_repo: ShareLinksRepository<'d>,
    files_repo: FilesRepository<'d>,
}

impl<'d> PublicSharesService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            shares_repo: ShareLinksRepository::new(db),
            files_repo: FilesRepository::new(db),
        }
    }

    /// Load share by token; 404 if missing / revoked / expired.
    pub async fn load_available(&self, token: &str) -> SarcaResult<ShareLink> {
        let link = self.shares_repo.get_by_token(token).await?;
        if link.is_unavailable() {
            return Err(SarcaError::DoesNotExist("share link".to_owned()));
        }
        Ok(link)
    }

    pub async fn metadata(&self, link: &ShareLink) -> SarcaResult<PublicShareMetaSchema> {
        let (name, is_file, size) = if link.is_folder() {
            let name = folder_basename(&link.path);
            let prefix = &link.path;
            // Confirm folder still exists (marker or children).
            let marker_ok = self
                .files_repo
                .get_file_by_path(prefix, link.storage_id)
                .await
                .is_ok();
            let files = self
                .files_repo
                .list_uploaded_files_under(link.storage_id, prefix)
                .await?;
            if !marker_ok && files.is_empty() {
                return Err(SarcaError::DoesNotExist("folder".to_owned()));
            }
            let size = self
                .files_repo
                .sum_uploaded_size_under(link.storage_id, prefix)
                .await
                .unwrap_or(0);
            (name, false, size)
        } else {
            let file = self
                .files_repo
                .get_file_by_path(&link.path, link.storage_id)
                .await?;
            if !file.is_uploaded {
                return Err(SarcaError::DoesNotExist("file".to_owned()));
            }
            let name = file
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&file.path)
                .to_string();
            (name, true, file.size)
        };

        Ok(PublicShareMetaSchema {
            path: link.path.clone(),
            name,
            is_file,
            size,
            has_password: link.has_password(),
        })
    }

    pub async fn verify_password(&self, link: &ShareLink, password: &str) -> SarcaResult<()> {
        let Some(hash) = link.password_hash.as_deref() else {
            return Ok(());
        };
        PasswordManager::verify(password, hash)
    }

    /// List children under share root + relative path. Paths in response are relative to share root.
    pub async fn tree(
        &self,
        link: &ShareLink,
        relative: &str,
    ) -> SarcaResult<Vec<FSElement>> {
        if !link.is_folder() {
            return Err(SarcaError::InvalidPath);
        }
        let abs_prefix = resolve_under_share(&link.path, relative, true)?;
        // list_dir expects prefix without forcing trailing slash input oddly —
        // pass without trailing slash except empty; it adds `/`.
        let list_prefix = abs_prefix.trim_end_matches('/');
        let elements = self
            .files_repo
            .list_dir(link.storage_id, list_prefix)
            .await?;

        let root = &link.path;
        Ok(elements
            .into_iter()
            .filter_map(|el| {
                let rel = el.path.strip_prefix(root)?.to_string();
                // Keep folder trailing slash in relative form when applicable.
                let (path, name) = if el.is_file {
                    (rel.clone(), el.name)
                } else {
                    let p = if rel.ends_with('/') {
                        rel
                    } else {
                        format!("{rel}/")
                    };
                    let name = folder_basename(&p);
                    (p, name)
                };
                Some(FSElement {
                    path,
                    name,
                    size: el.size,
                    is_file: el.is_file,
                    has_thumb: el.has_thumb,
                })
            })
            .collect())
    }

    /// Resolve a guest relative path to an absolute storage path under the share.
    pub fn resolve_file_path(&self, link: &ShareLink, relative: &str) -> SarcaResult<String> {
        if link.is_folder() {
            let abs = resolve_under_share(&link.path, relative, false)?;
            if abs.ends_with('/') {
                return Err(SarcaError::InvalidPath);
            }
            Ok(abs)
        } else {
            // File share: empty or matching basename only.
            let rel = relative.trim_start_matches('/');
            if rel.is_empty()
                || rel == link.path.rsplit('/').next().unwrap_or(&link.path)
                || rel == link.path
            {
                Ok(link.path.clone())
            } else {
                Err(SarcaError::DoesNotExist("file".to_owned()))
            }
        }
    }

    pub fn resolve_folder_zip_path(&self, link: &ShareLink) -> SarcaResult<String> {
        if !link.is_folder() {
            return Err(SarcaError::InvalidPath);
        }
        Ok(link.path.clone())
    }

    /// Cookie Max-Age seconds: min(24h, remaining link lifetime).
    pub fn unlock_max_age_secs(link: &ShareLink) -> u64 {
        const DAY: i64 = 24 * 60 * 60;
        let until_day = ChronoDuration::seconds(DAY);
        let max = match link.expires_at {
            Some(exp) => {
                let remaining = exp - Utc::now();
                if remaining <= ChronoDuration::zero() {
                    ChronoDuration::seconds(0)
                } else if remaining < until_day {
                    remaining
                } else {
                    until_day
                }
            }
            None => until_day,
        };
        max.num_seconds().max(0) as u64
    }
}

fn folder_basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

/// Normalize create-share target: no leading slash, no `..`, folders end with `/`.
fn normalize_share_target_path(path: &str) -> SarcaResult<String> {
    let path = path.trim().trim_start_matches('/');
    if path.is_empty() || path.contains("//") || path_has_dot_dot(path) {
        return Err(SarcaError::InvalidPath);
    }
    Ok(path.to_string())
}

fn path_has_dot_dot(path: &str) -> bool {
    path.split('/').any(|seg| seg == "..")
}

/// Join share root (folder ending in `/`) with a relative guest path. Reject escape.
fn resolve_under_share(
    share_root: &str,
    relative: &str,
    allow_dir: bool,
) -> SarcaResult<String> {
    if !share_root.ends_with('/') {
        return Err(SarcaError::InvalidPath);
    }
    let rel = relative.trim().trim_start_matches('/');
    if rel.contains("//") || path_has_dot_dot(rel) || rel.starts_with('/') {
        return Err(SarcaError::InvalidPath);
    }
    if rel.is_empty() {
        return Ok(share_root.to_string());
    }
    let joined = format!("{share_root}{rel}");
    // Ensure still under root (no sneaky absolute after join).
    if !joined.starts_with(share_root) {
        return Err(SarcaError::InvalidPath);
    }
    if !allow_dir && joined.ends_with('/') {
        return Err(SarcaError::InvalidPath);
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_rejects_dot_dot() {
        assert!(resolve_under_share("docs/", "../etc", false).is_err());
        assert!(resolve_under_share("docs/", "a/../../x", false).is_err());
    }

    #[test]
    fn resolve_joins_relative() {
        assert_eq!(
            resolve_under_share("docs/folder/", "a/b.txt", false).unwrap(),
            "docs/folder/a/b.txt"
        );
        assert_eq!(
            resolve_under_share("docs/folder/", "", true).unwrap(),
            "docs/folder/"
        );
    }
}
