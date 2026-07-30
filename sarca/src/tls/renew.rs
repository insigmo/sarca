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
    let mut reader = Cursor::new(cert_pem.as_bytes());
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| acme::AcmeError::InvalidCert)?;

    let cert_der = certs.first().ok_or(acme::AcmeError::InvalidCert)?;
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(cert_der.as_ref())
        .map_err(|_| acme::AcmeError::InvalidCert)?;
    let ts = cert.validity().not_after.timestamp();
    DateTime::from_timestamp(ts, 0).ok_or(acme::AcmeError::InvalidCert)
}

/// Background task: sleep until [`renew_at`], re-issue via ACME, hot-reload TCP TLS.
pub fn spawn_renewal_task(issuer: InstantAcmeIssuer, cert_store: CertStore, runtime: TlsRuntime) {
    tokio::spawn(
        async move {
            loop {
                let sleep_until = match cert_store.load_cert().await {
                    Ok(Some(pem)) => parse_not_after(&pem).ok().map(renew_at),
                    _ => None,
                };

                let Some(due_at) = sleep_until else {
                    tokio::time::sleep(Duration::from_hours(1)).await;
                    continue;
                };

                let now = Utc::now();
                if due_at > now {
                    let delay = (due_at - now).to_std().unwrap_or_else(|_| Duration::from_mins(1));
                    tracing::info!("ACME renewal scheduled at {due_at}");
                    tokio::time::sleep(delay).await;
                }

                tracing::info!("ACME certificate renewal starting");
                match renew_once(&issuer, &cert_store, &runtime).await {
                    Ok(()) => tracing::info!("ACME certificate renewed successfully"),
                    Err(e) => tracing::error!("ACME renewal failed: {e}"),
                }

                tokio::time::sleep(Duration::from_mins(5)).await;
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
    runtime.reload_tcp_material(&material);
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
    fn parse_not_after_from_self_signed_pem() {
        let cert = rcgen::generate_simple_self_signed(vec!["test.local".into()]).unwrap();
        let cert_pem = cert.cert.pem();
        let not_after = parse_not_after(&cert_pem).expect("parse not_after");
        assert!(not_after > Utc::now());
    }
}
