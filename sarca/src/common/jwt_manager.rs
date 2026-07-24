use std::{
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{SarcaError, SarcaResult};

pub const TOKEN_TYPE_ACCESS: &str = "access";
pub const TOKEN_TYPE_REFRESH: &str = "refresh";
pub const TOKEN_TYPE_SHARE_UNLOCK: &str = "share_unlock";

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    pub(self) sub: String,
    pub(self) email: String,
    pub(self) exp: usize,
    #[serde(default)]
    pub(self) token_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ShareUnlockClaims {
    /// Share link opaque token.
    pub(self) sub: String,
    pub(self) exp: usize,
    pub(self) token_type: String,
}

#[derive(Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub email: String,
}

impl AuthUser {
    pub fn new(id: Uuid, email: String) -> Self {
        Self {
            id,
            email,
        }
    }
}

pub struct JWTManager;

impl JWTManager {
    pub fn generate(
        user: AuthUser,
        expire_in: Duration,
        secret_key: &str,
        token_type: &str,
    ) -> String {
        let expire_date = SystemTime::now() + expire_in;
        let expire_timestamp = expire_date.duration_since(UNIX_EPOCH).unwrap().as_secs() as usize;
        let claims = Claims {
            sub: user.id.into(),
            email: user.email,
            exp: expire_timestamp,
            token_type: Some(token_type.to_owned()),
        };
        let key = EncodingKey::from_secret(secret_key.as_bytes());

        encode(&Header::default(), &claims, &key).unwrap()
    }

    pub fn validate(token: &str, secret_key: &str) -> SarcaResult<AuthUser> {
        Self::validate_with_type(token, secret_key, TOKEN_TYPE_ACCESS)
    }

    pub fn validate_refresh(token: &str, secret_key: &str) -> SarcaResult<AuthUser> {
        Self::validate_with_type(token, secret_key, TOKEN_TYPE_REFRESH)
    }

    fn validate_with_type(
        token: &str,
        secret_key: &str,
        expected_type: &str,
    ) -> SarcaResult<AuthUser> {
        let validation = Validation::new(Algorithm::HS256);
        let decoding_key = DecodingKey::from_secret(secret_key.as_bytes());

        decode::<Claims>(token, &decoding_key, &validation)
            .map_err(|_| SarcaError::NotAuthenticated)
            .and_then(|token_data| {
                let token_type =
                    token_data.claims.token_type.as_deref().unwrap_or(TOKEN_TYPE_ACCESS);
                if token_type != expected_type {
                    return Err(SarcaError::NotAuthenticated);
                }
                let id = token_data.claims.sub;
                let id = Uuid::from_str(&id).unwrap(); // token is valid so uuid is too
                Ok(AuthUser::new(id, token_data.claims.email))
            })
    }

    /// Short-lived unlock JWT for a password-protected share (stored in `HttpOnly` cookie).
    pub fn generate_share_unlock(
        share_token: &str,
        expire_in: Duration,
        secret_key: &str,
    ) -> String {
        let expire_date = SystemTime::now() + expire_in;
        let expire_timestamp = expire_date.duration_since(UNIX_EPOCH).unwrap().as_secs() as usize;
        let claims = ShareUnlockClaims {
            sub: share_token.to_owned(),
            exp: expire_timestamp,
            token_type: TOKEN_TYPE_SHARE_UNLOCK.to_owned(),
        };
        let key = EncodingKey::from_secret(secret_key.as_bytes());
        encode(&Header::default(), &claims, &key).unwrap()
    }

    /// Returns Ok if the unlock JWT is valid for `share_token`.
    pub fn validate_share_unlock(
        unlock_jwt: &str,
        share_token: &str,
        secret_key: &str,
    ) -> SarcaResult<()> {
        let validation = Validation::new(Algorithm::HS256);
        let decoding_key = DecodingKey::from_secret(secret_key.as_bytes());

        let token_data = decode::<ShareUnlockClaims>(unlock_jwt, &decoding_key, &validation)
            .map_err(|_| SarcaError::NotAuthenticated)?;

        if token_data.claims.token_type != TOKEN_TYPE_SHARE_UNLOCK {
            return Err(SarcaError::NotAuthenticated);
        }
        if token_data.claims.sub != share_token {
            return Err(SarcaError::NotAuthenticated);
        }
        Ok(())
    }
}
