use pwhash::bcrypt;

use crate::errors::{SarcaError, SarcaResult};

pub struct PasswordManager;

impl PasswordManager {
    pub fn generate(password: &str) -> SarcaResult<String> {
        bcrypt::hash(password).map_err(|e| {
            tracing::error!("{e}");
            SarcaError::Unknown
        })
    }

    pub fn verify(password: &str, hash: &str) -> SarcaResult<()> {
        if bcrypt::verify(password, hash) {
            Ok(())
        } else {
            Err(SarcaError::NotAuthenticated)
        }
    }
}
