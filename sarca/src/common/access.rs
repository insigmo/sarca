use uuid::Uuid;

use crate::{
    errors::{SarcaError, SarcaResult},
    models::access::AccessType,
    repositories::access::AccessRepository,
};

pub async fn check_access(
    repo: &AccessRepository<'_>,
    user_id: Uuid,
    storage_id: Uuid,
    access_type: &AccessType,
) -> SarcaResult<()> {
    if repo.has_access(user_id, storage_id, access_type).await? {
        Ok(())
    } else {
        Err(SarcaError::DoesNotExist(format!("storage with id \"{storage_id}\"")))
    }
}
