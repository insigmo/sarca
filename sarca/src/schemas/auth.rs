use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct LoginSchema {
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct RefreshSchema {
    pub refresh_token: String,
}

#[derive(Serialize)]
pub struct TokenSchema {
    pub access_token: String,
    pub refresh_token: String,
    pub email_verified: bool,
}

impl TokenSchema {
    pub fn new(access_token: String, refresh_token: String, email_verified: bool) -> Self {
        Self {
            access_token,
            refresh_token,
            email_verified,
        }
    }
}

#[derive(Serialize)]
pub struct MeSchema {
    pub email: String,
    pub email_verified: bool,
}

#[derive(Serialize)]
pub struct ProvidersSchema {
    pub google: bool,
    pub github: bool,
    pub smtp: bool,
}

#[derive(Deserialize)]
pub struct TokenBodySchema {
    pub token: String,
}

#[derive(Deserialize)]
pub struct ForgotPasswordSchema {
    pub email: String,
}

#[derive(Deserialize)]
pub struct ResetPasswordSchema {
    pub token: String,
    pub new_password: String,
}

#[derive(Deserialize)]
pub struct OAuthExchangeSchema {
    pub code: String,
}
