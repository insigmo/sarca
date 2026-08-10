//! ACME certificate renewal scheduling.
//!
//! Fixes the "too many certificates already cert for this exact set of
//! identifiers" Let's Encrypt rate-limit error caused by the old logic,
//! which re-requested a certificate on a fixed 600s timer regardless of
//! whether the certificate on disk was still valid.
//!
//! New behaviour:
//!   1. Look at what's in the cert store (certs dir).
//!      - No certificate on disk            -> issue immediately.
//!      - Certificate present               -> parse `notAfter`.
//!   2. If the certificate is already expired, or has <= 1 day left, renew immediately.
//!   3. If it still has more than 1 day left, do NOT touch ACME at all. Instead schedule the next
//!      renewal attempt for exactly `notAfter - 1 day`, and sleep until then.
//!   4. If an issuance attempt fails (including ACME rate limiting), back off instead of retrying
//!      every 600s: use a capped exponential backoff (starting at a few minutes, capped at a few
//!      hours) so a persistent outage cannot keep re-triggering the rate limit counter.

use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::time::Instant;

use super::{
    acme::{AcmeError, InstantAcmeIssuer, IssuedCertificate, save_issued},
    serve::{TlsRuntime, parse_pem_material},
    store::CertStore,
};

/// Safety margin before expiry at which we consider a certificate due for renewal.
const RENEW_BEFORE_EXPIRY: chrono::Duration = chrono::Duration::days(1);

/// Minimum backoff after a failed issuance attempt.
const MIN_RETRY_BACKOFF: Duration = Duration::from_mins(5);

/// Ceiling for the backoff after repeated failures, so a persistent outage
/// still checks in periodically without hammering the ACME server.
const MAX_RETRY_BACKOFF: Duration = Duration::from_hours(6);

/// Parse the `notAfter` field out of a PEM certificate.
pub fn parse_not_after(cert_pem: &str) -> Result<DateTime<Utc>, AcmeError> {
    use x509_parser::{pem::parse_x509_pem, prelude::FromDer};

    let (_, pem) = parse_x509_pem(cert_pem.as_bytes()).map_err(|_| AcmeError::InvalidCert)?;
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(&pem.contents)
        .map_err(|_| AcmeError::InvalidCert)?;
    let ts = cert.validity().not_after.timestamp();
    DateTime::<Utc>::from_timestamp(ts, 0).ok_or(AcmeError::InvalidCert)
}

/// Decide when the next renewal attempt should happen for a certificate
/// whose expiry is `not_after`. Never returns a time in the past relative
pub fn renew_at(not_after: DateTime<Utc>, now: DateTime<Utc>) -> DateTime<Utc> {
    let target = not_after - RENEW_BEFORE_EXPIRY;
    if target <= now { now } else { target }
}

/// True if a certificate expiring at `not_after` needs to be renewed right now.
fn due_for_renewal(not_after: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    not_after - now <= RENEW_BEFORE_EXPIRY
}

/// True when a PEM certificate is self-signed, i.e. its issuer equals its
/// subject.
///
/// The boot path calls `load_or_generate_material`, which writes a self-signed
/// placeholder into the same store so the server can serve HTTPS before any CA
/// has answered. Without this check the renewal task read that placeholder,
/// saw a `notAfter` decades away, decided nothing was due and went to sleep —
/// so a server with `SARCA_ACME=1` served a self-signed certificate forever and
/// never once contacted the CA.
fn is_self_signed(cert_pem: &str) -> bool {
    use x509_parser::{pem::parse_x509_pem, prelude::FromDer};

    let Ok((_, pem)) = parse_x509_pem(cert_pem.as_bytes()) else {
        return false;
    };
    let Ok((_, cert)) = x509_parser::certificate::X509Certificate::from_der(&pem.contents) else {
        return false;
    };
    cert.issuer() == cert.subject()
}

/// Read the `notAfter` of the stored CA-issued certificate, if there is one.
///
/// Returns `None` when nothing is stored or when the stored material is only
/// the self-signed placeholder, both of which mean "ask the CA now".
async fn stored_not_after(store: &CertStore) -> Option<DateTime<Utc>> {
    let cert_pem = CertStore::load_pem_at(&store.cert_path()).await.ok()??;
    if is_self_signed(&cert_pem) {
        return None;
    }
    parse_not_after(&cert_pem).ok()
}

/// One issuance attempt: request a certificate and persist it.
/// `InstantAcmeIssuer::issue()` already knows the directory URL, identity,
/// challenge store, ACME account path, root CA and private key path (they
/// live inside its own `AcmeConfig`), so renewal doesn't need to touch any
/// of that itself.
async fn issue_and_save(
    issuer: &InstantAcmeIssuer,
    store: &CertStore,
) -> Result<IssuedCertificate, AcmeError> {
    let cert = issuer.issue().await?;
    save_issued(store, &cert).await?;
    let not_after = cert.not_after;
    tracing::info!("ACME certificate issued (not_after={not_after})");
    Ok(cert)
}

/// Background task: keeps the on-disk certificate valid without hammering
/// the ACME server. Runs forever; spawn with `tokio::spawn`.
pub async fn spawn_renewal_task(issuer: InstantAcmeIssuer, store: CertStore, runtime: TlsRuntime) {
    tokio::spawn(async move {
        let mut failure_backoff = MIN_RETRY_BACKOFF;
        let renew_signal = runtime.renew_signal();

        loop {
            let now = Utc::now();
            let existing_not_after = stored_not_after(&store).await;

            let should_issue_now =
                existing_not_after.is_none_or(|not_after| due_for_renewal(not_after, now));

            if should_issue_now {
                match issue_and_save(&issuer, &store).await {
                    Ok(cert) => {
                        // The listeners were started with whatever material was
                        // on disk at boot (usually the self-signed placeholder),
                        // and only the resolver can hand them the new chain.
                        // Skipping this left clients seeing a self-signed
                        // certificate until the next process restart.
                        match parse_pem_material(&cert.cert_pem, &cert.key_pem) {
                            Ok(material) => {
                                if let Err(e) = runtime.reload_material(&material) {
                                    tracing::error!("failed to install renewed certificate: {e}");
                                }
                            },
                            Err(e) => {
                                tracing::error!("renewed certificate is unusable: {e}");
                            },
                        }

                        // Success resets the failure backoff and schedules
                        // the next check at not_after - 1 day.
                        failure_backoff = MIN_RETRY_BACKOFF;
                        let sleep_until = renew_at(cert.not_after, Utc::now());
                        sleep_until_utc(sleep_until, &renew_signal).await;
                        continue;
                    },
                    Err(e) => {
                        tracing::error!("ACME issuance failed: {e}");
                        tracing::info!(
                            "next ACME issuance attempt in {}s",
                            failure_backoff.as_secs()
                        );
                        tokio::select! {
                            () = tokio::time::sleep(failure_backoff) => {},
                            () = renew_signal.notified() => {},
                        }
                        failure_backoff =
                            (failure_backoff.saturating_mul(2)).min(MAX_RETRY_BACKOFF);
                        continue;
                    },
                }
            }

            // Certificate is still valid for more than 1 day: do nothing to
            // ACME, just sleep until the day-before-expiry checkpoint.
            let not_after = existing_not_after.expect("should_issue_now was false");
            let sleep_until = renew_at(not_after, now);
            sleep_until_utc(sleep_until, &renew_signal).await;
        }
    });
}

/// Sleep until a wall-clock UTC instant, converting to a monotonic Instant once.
///
/// Wakes early when something calls `TlsRuntime::request_renewal` — a public IP
/// change or repeated HTTP/3 handshake failures. Before this the signal was
/// never awaited by anyone, so both of those recovery paths were inert.
async fn sleep_until_utc(target: DateTime<Utc>, renew_signal: &tokio::sync::Notify) {
    let now = Utc::now();
    let dur = (target - now).to_std().unwrap_or(Duration::from_secs(0));
    // Re-check at least once a day even if something is off with clocks/
    // durations, so a miscalculation can't sleep forever.
    let dur = dur.min(Duration::from_hours(24));
    tokio::select! {
        () = tokio::time::sleep_until(Instant::now() + dur) => {},
        () = renew_signal.notified() => {},
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration as ChronoDuration;

    use super::*;

    #[test]
    fn renews_now_when_less_than_a_day_left() {
        let now = Utc::now();
        let not_after = now + ChronoDuration::hours(12);
        assert!(due_for_renewal(not_after, now));
        assert_eq!(renew_at(not_after, now), now);
    }

    #[test]
    fn renews_now_when_already_expired() {
        let now = Utc::now();
        let not_after = now - ChronoDuration::hours(1);
        assert!(due_for_renewal(not_after, now));
        assert_eq!(renew_at(not_after, now), now);
    }

    #[test]
    fn a_self_signed_placeholder_never_counts_as_an_issued_certificate() {
        let generated =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("self-signed");
        assert!(
            is_self_signed(&generated.cert.pem()),
            "the boot placeholder must be recognised, or ACME issuance never runs"
        );
    }

    #[test]
    fn schedules_for_one_day_before_expiry_when_plenty_of_time_left() {
        let now = Utc::now();
        let not_after = now + ChronoDuration::days(6);
        assert!(!due_for_renewal(not_after, now));
        let scheduled = renew_at(not_after, now);
        assert_eq!(scheduled, not_after - ChronoDuration::days(1));
    }
}
