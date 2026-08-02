use std::{env, net::SocketAddr, str::FromStr};

use super::{
    errors::{SarcaError, SarcaResult},
    tls::{TlsError, TlsIdentity, parse_tls_identity},
};

#[derive(Debug, Clone)]
pub struct Config {
    /// Path to the `SQLite` metadata database file.
    pub sqlite_path: String,
    pub port: u16,
    /// HTTPS listen address (HTTP/3 UDP + TLS TCP); dev default high port.
    pub https_addr: SocketAddr,
    /// ACME http-01 + redirect listener; dev default high port.
    pub acme_http_addr: SocketAddr,
    /// Certificate identity hostname (domain or IP); unset until install/ACME.
    pub tls_hostname: Option<String>,
    pub acme_directory: String,
    /// Directory for ACME account + issued PEM material.
    pub certs_dir: String,
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

    /// Max size of a single Telegram document chunk (official Bot API ≤20 MB).
    pub telegram_chunk_size_mb: u32,

    /// Chunk size for video uploads (≤20 MB; smaller chunks → progressive Range playback sooner).
    pub telegram_video_chunk_size_mb: u32,

    /// Public base URL for email links (verify / reset).
    pub public_base_url: String,

    pub smtp_host: Option<String>,
    pub smtp_port: u16,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_from: String,
    /// `starttls` | `none` | `tls`
    pub smtp_tls: String,

    /// Verbose (debug-level) tracing for requests and background jobs. `RUST_LOG` still wins.
    pub debug_log: bool,
}

impl Config {
    pub fn smtp_configured(&self) -> bool {
        self.smtp_host.is_some()
    }

    /// Default `SQLite` file location: `sarca.sqlite` inside `WORK_DIR`.
    pub fn default_sqlite_path(work_dir: &str) -> String {
        format!("{}/sarca.sqlite", work_dir.trim_end_matches('/'))
    }

    /// Default PEM store: `certs/` inside `WORK_DIR`.
    pub fn default_certs_dir(work_dir: &str) -> String {
        format!("{}/certs", work_dir.trim_end_matches('/'))
    }

    /// Parsed TLS identity when `TLS_HOSTNAME` is set.
    pub fn tls_identity(&self) -> Result<Option<TlsIdentity>, TlsError> {
        self.tls_hostname.as_deref().map_or(Ok(None), |host| parse_tls_identity(host).map(Some))
    }

    pub fn new() -> SarcaResult<Self> {
        let work_dir: String = Self::get_env_var_with_default("WORK_DIR", "work".to_owned())?;
        let sqlite_path = Self::get_optional_env_var("SQLITE_PATH")
            .unwrap_or_else(|| Self::default_sqlite_path(&work_dir));
        let port = Self::get_env_var("PORT")?;
        let https_addr = Self::get_env_var_with_default(
            "HTTPS_ADDR",
            "0.0.0.0:8443".parse().expect("valid addr"),
        )?;
        let acme_http_addr = Self::get_env_var_with_default(
            "ACME_HTTP_ADDR",
            "0.0.0.0:8080".parse().expect("valid addr"),
        )?;
        let tls_hostname = Self::get_optional_env_var("TLS_HOSTNAME");
        let acme_directory = Self::get_env_var_with_default(
            "ACME_DIRECTORY",
            "https://acme-v02.api.letsencrypt.org/directory".to_owned(),
        )?;
        let certs_dir = Self::get_optional_env_var("CERTS_DIR")
            .unwrap_or_else(|| Self::default_certs_dir(&work_dir));
        let workers = Self::get_env_var("WORKERS")?;
        let channel_capacity = Self::get_env_var("CHANNEL_CAPACITY")?;
        let superuser_email = Self::get_env_var("SUPERUSER_EMAIL")?;
        let superuser_pass = Self::get_env_var("SUPERUSER_PASS")?;
        let access_token_expire_in_secs = Self::get_env_var("ACCESS_TOKEN_EXPIRE_IN_SECS")?;
        let refresh_token_expire_in_days = Self::get_env_var("REFRESH_TOKEN_EXPIRE_IN_DAYS")?;
        let secret_key = Self::get_env_var("SECRET_KEY")?;
        let telegram_api_base_url = Self::get_env_var_with_default(
            "TELEGRAM_API_BASE_URL",
            "https://api.telegram.org".to_owned(),
        )?;
        let telegram_rate_limit = Self::get_env_var_with_default("TELEGRAM_RATE_LIMIT", 18)?;
        let telegram_chunk_size_mb =
            Self::get_env_var_with_default("TELEGRAM_CHUNK_SIZE_MB", 20u32)?;
        let telegram_video_chunk_size_mb =
            Self::get_env_var_with_default("TELEGRAM_VIDEO_CHUNK_SIZE_MB", 20u32)?;

        let public_base_url =
            Self::get_env_var_with_default("PUBLIC_BASE_URL", format!("http://127.0.0.1:{port}"))?;
        let smtp_host = Self::get_optional_env_var("SMTP_HOST");
        let smtp_port = Self::get_env_var_with_default("SMTP_PORT", 587u16)?;
        let smtp_username = Self::get_optional_env_var("SMTP_USERNAME");
        let smtp_password = Self::get_optional_env_var("SMTP_PASSWORD");
        let smtp_from =
            Self::get_env_var_with_default("SMTP_FROM", "Sarca <noreply@example.com>".to_owned())?;
        let smtp_tls = Self::get_env_var_with_default("SMTP_TLS", "starttls".to_owned())?;
        let debug_log = Self::get_optional_env_var("DEBUG_LOG")
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

        Ok(Self {
            sqlite_path,
            port,
            https_addr,
            acme_http_addr,
            tls_hostname,
            acme_directory,
            certs_dir,
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
            telegram_video_chunk_size_mb,
            public_base_url,
            smtp_host,
            smtp_port,
            smtp_username,
            smtp_password,
            smtp_from,
            smtp_tls,
            debug_log,
        })
    }

    /// Bytes per Telegram chunk for this file (video → smaller chunks).
    pub fn chunk_size_bytes_for_file(&self, path: &str, content_type: Option<&str>) -> i64 {
        let mb = if crate::models::files::is_video(path, content_type) {
            self.telegram_video_chunk_size_mb
        } else {
            self.telegram_chunk_size_mb
        };
        i64::from(mb).saturating_mul(1024 * 1024).max(1)
    }

    /// Default (non-video) chunk size in bytes — used when a file row has no `chunk_size_bytes`.
    pub fn default_chunk_size_bytes(&self) -> u64 {
        u64::from(self.telegram_chunk_size_mb).saturating_mul(1024 * 1024).max(1)
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
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_required() {
        for k in [
            "SQLITE_PATH",
            "PORT",
            "WORKERS",
            "CHANNEL_CAPACITY",
            "SUPERUSER_EMAIL",
            "SUPERUSER_PASS",
            "ACCESS_TOKEN_EXPIRE_IN_SECS",
            "REFRESH_TOKEN_EXPIRE_IN_DAYS",
            "SECRET_KEY",
            "TELEGRAM_API_BASE_URL",
            "TELEGRAM_RATE_LIMIT",
            "TELEGRAM_CHUNK_SIZE_MB",
            "TELEGRAM_VIDEO_CHUNK_SIZE_MB",
            "WORK_DIR",
            "HTTPS_ADDR",
            "ACME_HTTP_ADDR",
            "TLS_HOSTNAME",
            "ACME_DIRECTORY",
            "CERTS_DIR",
            "SARCA_PLAIN_HTTP",
            "SARCA_ACME",
        ] {
            env::remove_var(k);
        }
    }

    fn set_required() {
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
    fn default_sqlite_path_sits_beside_work_dir() {
        assert_eq!(Config::default_sqlite_path("work"), "work/sarca.sqlite");
        assert!(Config::default_sqlite_path("/var/lib/sarca/work").ends_with("sarca.sqlite"));
    }

    #[test]
    fn sqlite_path_from_env_overrides_default() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_required();
        set_required();
        env::set_var("SQLITE_PATH", "/tmp/custom.sqlite");
        let cfg = Config::new().unwrap();
        assert_eq!(cfg.sqlite_path, "/tmp/custom.sqlite");
        clear_required();
    }

    #[test]
    fn loads_port_from_env() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_required();
        set_required();
        let cfg = Config::new().unwrap();
        assert_eq!(cfg.port, 8001);
        assert_eq!(cfg.sqlite_path, "work/sarca.sqlite");
        clear_required();
    }

    #[test]
    fn missing_required_errors() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_required();
        let err = Config::new().unwrap_err();
        assert!(matches!(err, SarcaError::EnvConfigLoadingError(_)));
    }

    #[test]
    fn video_chunk_default_capped_at_20() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_required();
        set_required();
        let cfg = Config::new().unwrap();
        assert!(cfg.telegram_video_chunk_size_mb <= 20);
        assert!(cfg.telegram_chunk_size_mb <= 20);
        clear_required();
    }

    #[test]
    fn tls_addrs_and_certs_dir_defaults_for_dev() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_required();
        set_required();
        let cfg = Config::new().unwrap();
        assert_eq!(cfg.https_addr, "0.0.0.0:8443".parse().unwrap());
        assert_eq!(cfg.acme_http_addr, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(cfg.certs_dir, "work/certs");
        assert!(cfg.acme_directory.contains("letsencrypt.org"));
        assert!(cfg.tls_hostname.is_none());
        clear_required();
    }

    #[test]
    fn tls_hostname_parses_to_identity() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_required();
        set_required();
        env::set_var("TLS_HOSTNAME", "203.0.113.10");
        let cfg = Config::new().unwrap();
        let id = cfg.tls_identity().unwrap().unwrap();
        assert!(matches!(id, TlsIdentity::Ip(_)));
        clear_required();
    }
}
