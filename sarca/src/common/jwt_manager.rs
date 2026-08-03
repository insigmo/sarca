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
    /// Issue time. Compared against `users.sessions_valid_after` so that a
    /// password reset or an explicit logout can evict already-issued tokens,
    /// which are otherwise stateless and unrevocable.
    #[serde(default)]
    pub(self) iat: i64,
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
    /// `iat` of the token this identity came from; `0` when it was not built
    /// from a token (freshly issued identities).
    pub issued_at: i64,
}

impl AuthUser {
    pub fn new(id: Uuid, email: String) -> Self {
        Self {
            id,
            email,
            issued_at: 0,
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
        let now = SystemTime::now();
        let expire_date = now + expire_in;
        let expire_timestamp = expire_date.duration_since(UNIX_EPOCH).unwrap().as_secs() as usize;
        let claims = Claims {
            sub: user.id.into(),
            email: user.email,
            exp: expire_timestamp,
            token_type: Some(token_type.to_owned()),
            iat: now.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
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

    /// HS256 only, with `exp` and `sub` demanded rather than merely checked.
    ///
    /// Naming the algorithm keeps a token whose header says `alg: none` (or
    /// any asymmetric algorithm) from being accepted, and requiring the claims
    /// means a token that simply omits `exp` cannot slip through as
    /// "never expires".
    fn validation() -> Validation {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_required_spec_claims(&["exp", "sub"]);
        validation
    }

    fn validate_with_type(
        token: &str,
        secret_key: &str,
        expected_type: &str,
    ) -> SarcaResult<AuthUser> {
        let validation = Self::validation();
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
                let id = Uuid::from_str(&id).map_err(|_| SarcaError::NotAuthenticated)?;
                let mut auth = AuthUser::new(id, token_data.claims.email);
                auth.issued_at = token_data.claims.iat;
                Ok(auth)
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
        let validation = Self::validation();
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

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret-value";

    fn user() -> AuthUser {
        AuthUser::new(Uuid::nil(), "user@example.com".to_owned())
    }

    fn access_token() -> String {
        JWTManager::generate(user(), Duration::from_mins(1), SECRET, TOKEN_TYPE_ACCESS)
    }

    #[test]
    fn a_valid_access_token_round_trips() {
        let auth = JWTManager::validate(&access_token(), SECRET).expect("valid token");
        assert_eq!(auth.id, Uuid::nil());
        assert_eq!(auth.email, "user@example.com");
        assert!(auth.issued_at > 0, "iat must be carried through for session revocation");
    }

    #[test]
    fn another_secret_cannot_mint_a_token() {
        let forged =
            JWTManager::generate(user(), Duration::from_mins(1), "other-secret", TOKEN_TYPE_ACCESS);
        assert!(JWTManager::validate(&forged, SECRET).is_err());
    }

    #[test]
    fn an_expired_token_is_refused() {
        // Beyond the default 60s leeway.
        let claims = Claims {
            sub: Uuid::nil().into(),
            email: "user@example.com".to_owned(),
            exp: 1,
            token_type: Some(TOKEN_TYPE_ACCESS.to_owned()),
            iat: 0,
        };
        let token =
            encode(&Header::default(), &claims, &EncodingKey::from_secret(SECRET.as_bytes()))
                .unwrap();
        assert!(JWTManager::validate(&token, SECRET).is_err());
    }

    // A refresh token lives far longer than an access token, so letting one be
    // used as the other would stretch the access window to the refresh window.
    #[test]
    fn the_two_token_types_are_not_interchangeable() {
        let refresh =
            JWTManager::generate(user(), Duration::from_mins(1), SECRET, TOKEN_TYPE_REFRESH);
        assert!(JWTManager::validate(&refresh, SECRET).is_err());
        assert!(JWTManager::validate_refresh(&access_token(), SECRET).is_err());
    }

    #[test]
    fn an_unsigned_token_is_refused() {
        // The classic JWT bypass: header `{"alg":"none","typ":"JWT"}`, a
        // well-formed unexpired access-token body for the nil UUID, and an
        // empty signature.
        const ALG_NONE: &str = concat!(
            "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.",
            "eyJzdWIiOiIwMDAwMDAwMC0wMDAwLTAwMDAtMDAwMC0wMDAwMDAwMDAwMDAiLCJlbWFpbCI6InVzZXJAZXhh",
            "bXBsZS5jb20iLCJleHAiOjk5OTk5OTk5OTksInRva2VuX3R5cGUiOiJhY2Nlc3MiLCJpYXQiOjF9.",
        );
        assert!(JWTManager::validate(ALG_NONE, SECRET).is_err());
    }

    #[test]
    fn a_share_unlock_cookie_only_opens_its_own_share() {
        let jwt = JWTManager::generate_share_unlock("share-a", Duration::from_mins(1), SECRET);
        assert!(JWTManager::validate_share_unlock(&jwt, "share-a", SECRET).is_ok());
        assert!(JWTManager::validate_share_unlock(&jwt, "share-b", SECRET).is_err());
    }

    // An access token and an unlock cookie are signed with the same key, so the
    // `token_type` claim is the only thing keeping one from being replayed as
    // the other.
    #[test]
    fn an_access_token_is_not_a_share_unlock_cookie() {
        // `sub` is deliberately the share token, so `token_type` is the only
        // thing left that can reject it.
        let claims = Claims {
            sub: "share-a".to_owned(),
            email: "user@example.com".to_owned(),
            exp: 9_999_999_999,
            token_type: Some(TOKEN_TYPE_ACCESS.to_owned()),
            iat: 1,
        };
        let token =
            encode(&Header::default(), &claims, &EncodingKey::from_secret(SECRET.as_bytes()))
                .unwrap();
        assert!(JWTManager::validate_share_unlock(&token, "share-a", SECRET).is_err());
    }
}
