use crate::errors::SarcaError;

#[inline]
pub fn map_not_found(e: sqlx::Error, entity_name: &str) -> SarcaError {
    match e {
        sqlx::Error::RowNotFound => SarcaError::DoesNotExist(format!("such {entity_name}")),
        _ => {
            tracing::error!("{e}");
            SarcaError::Unknown
        }
    }
}
