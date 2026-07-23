use std::{env, str::FromStr};

use super::errors::{SarcaError, SarcaResult};

#[derive(Debug, Clone)]
pub struct Config {
    pub db_uri: String,
    pub db_uri_without_dbname: String,
    pub db_name: String,
    pub port: u16,
    pub workers: u16,
    pub channel_capacity: u16,
    pub superuser_email: String,
    pub superuser_pass: String,

    pub access_token_expire_in_secs: u32,
    pub refresh_token_expire_in_days: u16,
    pub secret_key: String,

    pub telegram_api_base_url: String,
    pub telegram_rate_limit: u8,

    /// Where to spool uploads and other temporary data.
    pub work_dir: String,

    /// Max size of a single Telegram document chunk.
    ///
    /// - Official Bot API has a practical 20MB download limitation via `getFile`.
    /// - Local Bot API can handle up to ~2GB per upload, so chunk size can be much larger.
    pub telegram_chunk_size_mb: u32,

    /// Optional bootstrap: bot token from @BotFather.
    pub telegram_bot_token: Option<String>,
    /// Optional bootstrap: channel id without `-100` prefix.
    pub telegram_channel_id: Option<i64>,
    /// Optional bootstrap: storage name to create for the superuser.
    pub storage_name: Option<String>,
}

impl Config {
    pub fn new() -> SarcaResult<Self> {
        let db_user: String = Self::get_env_var("DATABASE_USER")?;
        let db_password: String = Self::get_env_var("DATABASE_PASSWORD")?;
        let db_name: String = Self::get_env_var("DATABASE_NAME")?;
        let db_host: String = Self::get_env_var("DATABASE_HOST")?;
        let db_port: String = Self::get_env_var("DATABASE_PORT")?;
        let db_uri =
            { format!("postgres://{db_user}:{db_password}@{db_host}:{db_port}/{db_name}") };
        let db_uri_without_dbname =
            { format!("postgres://{db_user}:{db_password}@{db_host}:{db_port}") };
        let port = Self::get_env_var("PORT")?;
        let workers = Self::get_env_var("WORKERS")?;
        let channel_capacity = Self::get_env_var("CHANNEL_CAPACITY")?;
        let superuser_email = Self::get_env_var("SUPERUSER_EMAIL")?;
        let superuser_pass = Self::get_env_var("SUPERUSER_PASS")?;
        let access_token_expire_in_secs = Self::get_env_var("ACCESS_TOKEN_EXPIRE_IN_SECS")?;
        let refresh_token_expire_in_days = Self::get_env_var("REFRESH_TOKEN_EXPIRE_IN_DAYS")?;
        let secret_key = Self::get_env_var("SECRET_KEY")?;
        let telegram_local_api: bool =
            Self::get_env_var_with_default("TELEGRAM_LOCAL_API", false)?;
        let telegram_api_base_url: String = if telegram_local_api {
            Self::get_env_var_with_default("TELEGRAM_API_BASE_URL", "http://127.0.0.1:8081".to_owned())?
        } else {
            Self::get_env_var_with_default("TELEGRAM_API_BASE_URL", "https://api.telegram.org".to_owned())?
        };
        let telegram_rate_limit = Self::get_env_var_with_default("TELEGRAM_RATE_LIMIT", 18)?;

        let work_dir = Self::get_env_var_with_default("WORK_DIR", "work".to_owned())?;

        let default_chunk_mb = if telegram_api_base_url.contains("api.telegram.org") {
            20
        } else {
            // stay under the 2GB limit with some headroom
            1950
        };
        let telegram_chunk_size_mb =
            Self::get_env_var_with_default("TELEGRAM_CHUNK_SIZE_MB", default_chunk_mb)?;

        let telegram_bot_token = Self::get_optional_env_var("TELEGRAM_BOT_TOKEN");
        let telegram_channel_id = Self::get_optional_parsed_env_var("TELEGRAM_CHANNEL_ID")?;
        let storage_name = Self::get_optional_env_var("STORAGE_NAME");

        Ok(Self {
            db_uri,
            db_uri_without_dbname,
            db_name,
            port,
            workers,
            channel_capacity,
            superuser_email,
            superuser_pass,
            access_token_expire_in_secs,
            refresh_token_expire_in_days,
            secret_key,
            telegram_api_base_url,
            telegram_rate_limit,
            work_dir,
            telegram_chunk_size_mb,
            telegram_bot_token,
            telegram_channel_id,
            storage_name,
        })
    }

    #[inline]
    fn get_env_var<T: FromStr>(env_var: &str) -> SarcaResult<T> {
        env::var(env_var)
            .map_err(|_| SarcaError::EnvConfigLoadingError(env_var.to_owned()))?
            .parse::<T>()
            .map_err(|_| SarcaError::EnvVarParsingError(env_var.to_owned()))
    }

    #[inline]
    fn get_env_var_with_default<T: FromStr>(env_var: &str, default: T) -> SarcaResult<T> {
        let result = Self::get_env_var(env_var);

        if matches!(result, Err(SarcaError::EnvConfigLoadingError(_))) {
            return Ok(default);
        }

        result
    }

    /// Missing or blank env → `None`.
    #[inline]
    fn get_optional_env_var(env_var: &str) -> Option<String> {
        match env::var(env_var) {
            Ok(value) if !value.trim().is_empty() => Some(value),
            _ => None,
        }
    }

    /// Missing or blank env → `None`; non-empty but unparsable → error.
    #[inline]
    fn get_optional_parsed_env_var<T: FromStr>(env_var: &str) -> SarcaResult<Option<T>> {
        match Self::get_optional_env_var(env_var) {
            Some(value) => value
                .parse::<T>()
                .map(Some)
                .map_err(|_| SarcaError::EnvVarParsingError(env_var.to_owned())),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_required() {
        for k in [
            "DATABASE_USER",
            "DATABASE_PASSWORD",
            "DATABASE_NAME",
            "DATABASE_HOST",
            "DATABASE_PORT",
            "PORT",
            "WORKERS",
            "CHANNEL_CAPACITY",
            "SUPERUSER_EMAIL",
            "SUPERUSER_PASS",
            "ACCESS_TOKEN_EXPIRE_IN_SECS",
            "REFRESH_TOKEN_EXPIRE_IN_DAYS",
            "SECRET_KEY",
            "TELEGRAM_LOCAL_API",
            "TELEGRAM_API_BASE_URL",
            "TELEGRAM_RATE_LIMIT",
            "TELEGRAM_CHUNK_SIZE_MB",
            "WORK_DIR",
            "TELEGRAM_BOT_TOKEN",
            "TELEGRAM_CHANNEL_ID",
            "STORAGE_NAME",
        ] {
            env::remove_var(k);
        }
    }

    fn set_required() {
        env::set_var("DATABASE_USER", "sarca");
        env::set_var("DATABASE_PASSWORD", "sarca");
        env::set_var("DATABASE_NAME", "sarca");
        env::set_var("DATABASE_HOST", "127.0.0.1");
        env::set_var("DATABASE_PORT", "5432");
        env::set_var("PORT", "8001");
        env::set_var("WORKERS", "2");
        env::set_var("CHANNEL_CAPACITY", "8");
        env::set_var("SUPERUSER_EMAIL", "a@b.c");
        env::set_var("SUPERUSER_PASS", "pass");
        env::set_var("ACCESS_TOKEN_EXPIRE_IN_SECS", "1800");
        env::set_var("REFRESH_TOKEN_EXPIRE_IN_DAYS", "14");
        env::set_var("SECRET_KEY", "secret");
    }

    #[test]
    fn loads_port_from_env() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_required();
        set_required();
        let cfg = Config::new().unwrap();
        assert_eq!(cfg.port, 8001);
        assert!(cfg.db_uri.contains("127.0.0.1:5432/sarca"));
        assert!(cfg.telegram_bot_token.is_none());
        clear_required();
    }

    #[test]
    fn optional_bootstrap_vars() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_required();
        set_required();
        env::set_var("TELEGRAM_BOT_TOKEN", "tok");
        env::set_var("TELEGRAM_CHANNEL_ID", "123");
        env::set_var("STORAGE_NAME", "main");
        let cfg = Config::new().unwrap();
        assert_eq!(cfg.telegram_bot_token.as_deref(), Some("tok"));
        assert_eq!(cfg.telegram_channel_id, Some(123));
        assert_eq!(cfg.storage_name.as_deref(), Some("main"));
        clear_required();
    }

    #[test]
    fn blank_optional_is_none() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_required();
        set_required();
        env::set_var("TELEGRAM_BOT_TOKEN", "  ");
        env::set_var("STORAGE_NAME", "");
        let cfg = Config::new().unwrap();
        assert!(cfg.telegram_bot_token.is_none());
        assert!(cfg.storage_name.is_none());
        clear_required();
    }

    #[test]
    fn missing_required_errors() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_required();
        let err = Config::new().unwrap_err();
        assert!(matches!(err, SarcaError::EnvConfigLoadingError(_)));
    }
}
