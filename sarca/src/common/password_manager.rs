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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_roundtrip() {
        let hash = PasswordManager::generate("s3cret").unwrap();
        assert!(PasswordManager::verify("s3cret", &hash).is_ok());
        assert!(PasswordManager::verify("wrong", &hash).is_err());
    }
}
