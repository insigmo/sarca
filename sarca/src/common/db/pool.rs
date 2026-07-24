use std::time::Duration;

use sqlx::{PgPool, postgres::PgPoolOptions};

pub async fn get_pool(dsn: &str, max_connection: u32, timeout: Duration) -> Result<PgPool, String> {
    let connect =
        PgPoolOptions::new().max_connections(max_connection).acquire_timeout(timeout).connect(dsn);

    match tokio::time::timeout(timeout, connect).await {
        Ok(Ok(db)) => {
            tracing::debug!("established connection with database");
            Ok(db)
        },
        Ok(Err(e)) => Err(format!("database connection failed ({}): {e}", mask_dsn(dsn))),
        Err(_) => {
            Err(format!(
                "database connection timed out after {}s ({})",
                timeout.as_secs(),
                mask_dsn(dsn)
            ))
        },
    }
}

/// Hide password in <postgres://user:pass@host/db> for logs.
pub fn mask_dsn(dsn: &str) -> String {
    // postgres://user:password@host:port/db
    if let Some(scheme_sep) = dsn.find("://") {
        let scheme = &dsn[..scheme_sep + 3];
        let rest = &dsn[scheme_sep + 3..];
        if let Some(at) = rest.find('@') {
            let creds = &rest[..at];
            let host = &rest[at..];
            if let Some(colon) = creds.find(':') {
                let user = &creds[..colon];
                return format!("{scheme}{user}:***{host}");
            }
        }
    }
    dsn.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_password() {
        assert_eq!(
            mask_dsn("postgres://sarca:secret@127.0.0.1:5432/sarca"),
            "postgres://sarca:***@127.0.0.1:5432/sarca"
        );
    }

    #[test]
    fn leaves_dsn_without_password() {
        let dsn = "postgres://127.0.0.1/sarca";
        assert_eq!(mask_dsn(dsn), dsn);
    }
}
