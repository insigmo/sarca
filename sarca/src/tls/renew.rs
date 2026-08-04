use std::{io::Cursor, time::Duration};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tracing::Instrument;
use x509_parser::prelude::FromDer;

use super::{
    CertStore,
    TlsRuntime,
    acme::{self, InstantAcmeIssuer, IssuedCertificate, save_issued},
    parse_pem_material,
};

/// Schedule certificate renewal one day before `notAfter` (LE short-lived profile).
pub fn renew_at(not_after: DateTime<Utc>) -> DateTime<Utc> {
    not_after - ChronoDuration::days(1)
}

/// Parse `notAfter` from the first PEM certificate in `cert_pem`.
pub fn parse_not_after(cert_pem: &str) -> Result<DateTime<Utc>, acme::AcmeError> {
    with_first_cert(cert_pem, |cert| {
        let ts = cert.validity().not_after.timestamp();
        DateTime::from_timestamp(ts, 0).ok_or(acme::AcmeError::InvalidCert)
    })
}

/// Whether the stored certificate is our own self-signed fallback.
///
/// A self-signed cert means ACME issuance never succeeded (or the stored ACME
/// cert was replaced), so its `notAfter` is meaningless as a renewal schedule:
/// `rcgen` dates run to the year 4096. Detect it and retry on a backoff instead.
pub fn is_self_signed(cert_pem: &str) -> bool {
    with_first_cert(cert_pem, |cert| Ok(cert.issuer() == cert.subject())).unwrap_or(false)
}

fn with_first_cert<T>(
    cert_pem: &str,
    f: impl FnOnce(&x509_parser::certificate::X509Certificate<'_>) -> Result<T, acme::AcmeError>,
) -> Result<T, acme::AcmeError> {
    let mut reader = Cursor::new(cert_pem.as_bytes());
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| acme::AcmeError::InvalidCert)?;

    let cert_der = certs.first().ok_or(acme::AcmeError::InvalidCert)?;
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(cert_der.as_ref())
        .map_err(|_| acme::AcmeError::InvalidCert)?;
    f(&cert)
}

/// Shortest gap between two on-demand re-issues.
///
/// Failing HTTP/3 handshakes and public IP flapping both poke the renewal task;
/// this keeps that from turning into a Let's Encrypt rate-limit ban.
const MIN_REISSUE_INTERVAL: Duration = Duration::from_mins(10);

/// First retry delay after a failed issuance (or while running on self-signed).
const RETRY_MIN: Duration = Duration::from_mins(5);

/// Ceiling for the retry backoff, so a long outage still gets hourly attempts.
const RETRY_MAX: Duration = Duration::from_hours(1);

/// Next retry delay: double until [`RETRY_MAX`].
fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(RETRY_MAX)
}

/// How long to wait before the next issuance attempt.
///
/// A valid ACME certificate schedules by [`renew_at`]; anything else (missing,
/// unparseable, or our self-signed fallback) retries on `backoff`.
pub fn schedule_delay(cert_pem: Option<&str>, backoff: Duration, now: DateTime<Utc>) -> Duration {
    let Some(pem) = cert_pem else {
        return backoff;
    };
    if is_self_signed(pem) {
        return backoff;
    }
    let Ok(not_after) = parse_not_after(pem) else {
        return backoff;
    };
    let due_at = renew_at(not_after);
    if due_at <= now {
        return Duration::ZERO;
    }
    (due_at - now).to_std().unwrap_or(backoff)
}

/// Background task: sleep until [`renew_at`] (or until something asks for an
/// early renewal), re-issue via ACME, hot-reload TCP TLS and HTTP/3.
pub fn spawn_renewal_task(issuer: InstantAcmeIssuer, cert_store: CertStore, runtime: TlsRuntime) {
    let signal = runtime.renew_signal();
    tokio::spawn(
        async move {
            let mut last_issue = tokio::time::Instant::now();
            let mut backoff = RETRY_MIN;
            let mut last_failed = false;
            loop {
                let stored = cert_store.load_cert().await.ok().flatten();
                let mut delay = schedule_delay(stored.as_deref(), backoff, Utc::now());
                // A failed attempt against a still-valid but due certificate
                // would otherwise schedule zero delay and spin.
                if last_failed {
                    delay = delay.max(backoff);
                }
                if delay > Duration::ZERO {
                    tracing::info!("next ACME issuance attempt in {}s", delay.as_secs());
                }

                let on_demand = tokio::select! {
                    () = tokio::time::sleep(delay) => false,
                    () = signal.notified() => true,
                };

                if on_demand {
                    let since = last_issue.elapsed();
                    if since < MIN_REISSUE_INTERVAL {
                        tracing::info!(
                            "early renewal requested but last issue was {}s ago; waiting",
                            since.as_secs()
                        );
                        tokio::time::sleep(MIN_REISSUE_INTERVAL.saturating_sub(since)).await;
                    }
                }

                last_issue = tokio::time::Instant::now();
                tracing::info!("ACME certificate renewal starting");
                match renew_once(&issuer, &cert_store, &runtime).await {
                    Ok(()) => {
                        tracing::info!("ACME certificate renewed successfully");
                        backoff = RETRY_MIN;
                        last_failed = false;
                    },
                    Err(e) => {
                        backoff = next_backoff(backoff);
                        last_failed = true;
                        tracing::error!(
                            "ACME renewal failed: {e}; retrying in {}s",
                            backoff.as_secs()
                        );
                    },
                }
            }
        }
        .instrument(tracing::info_span!("acme_renewal")),
    );
}

async fn renew_once(
    issuer: &InstantAcmeIssuer,
    cert_store: &CertStore,
    runtime: &TlsRuntime,
) -> Result<(), acme::AcmeError> {
    let bundle = issuer.issue().await?;
    save_issued(cert_store, &bundle).await?;
    apply_renewed_material(runtime, &bundle)?;
    Ok(())
}

fn apply_renewed_material(
    runtime: &TlsRuntime,
    issued: &IssuedCertificate,
) -> Result<(), acme::AcmeError> {
    let material = parse_pem_material(&issued.cert_pem, &issued.key_pem)
        .map_err(|_| acme::AcmeError::InvalidCert)?;
    runtime.reload_material(&material).map_err(|_| acme::AcmeError::InvalidCert)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn renew_at_is_one_day_before_not_after() {
        let not_after = Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
        let renew = renew_at(not_after);
        assert_eq!(renew, Utc.with_ymd_and_hms(2026, 7, 31, 12, 0, 0).unwrap());
    }

    #[test]
    fn self_signed_is_detected() {
        let cert = rcgen::generate_simple_self_signed(vec!["test.local".into()]).unwrap();
        assert!(is_self_signed(&cert.cert.pem()));
    }

    #[test]
    fn self_signed_cert_schedules_backoff_not_year_4095() {
        let cert = rcgen::generate_simple_self_signed(vec!["test.local".into()]).unwrap();
        let delay = schedule_delay(Some(&cert.cert.pem()), RETRY_MIN, Utc::now());
        assert_eq!(delay, RETRY_MIN);
    }

    #[test]
    fn missing_or_garbage_cert_schedules_backoff() {
        assert_eq!(schedule_delay(None, RETRY_MIN, Utc::now()), RETRY_MIN);
        assert_eq!(schedule_delay(Some("not a pem"), RETRY_MIN, Utc::now()), RETRY_MIN);
    }

    #[test]
    fn backoff_doubles_up_to_ceiling() {
        assert_eq!(next_backoff(RETRY_MIN), RETRY_MIN * 2);
        assert_eq!(next_backoff(RETRY_MAX), RETRY_MAX);
        assert_eq!(next_backoff(RETRY_MAX / 2 + Duration::from_secs(1)), RETRY_MAX);
    }

    #[test]
    fn parse_not_after_from_self_signed_pem() {
        let cert = rcgen::generate_simple_self_signed(vec!["test.local".into()]).unwrap();
        let cert_pem = cert.cert.pem();
        let not_after = parse_not_after(&cert_pem).expect("parse not_after");
        assert!(not_after > Utc::now());
    }
}
