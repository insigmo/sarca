mod acme;
mod renew;
mod serve;
mod store;

use std::net::IpAddr;

pub use acme::{
    AcmeChallengeStore,
    AcmeConfig,
    AcmeError,
    AcmeIssuer,
    InstantAcmeIssuer,
    IssuedCertificate,
    SHORTLIVED_PROFILE,
    StubAcmeIssuer,
    acme_enabled,
    identity_to_identifier,
    register_challenge,
    save_issued,
};
pub use renew::{parse_not_after, renew_at, spawn_renewal_task};
pub use serve::{
    ChallengeStore,
    TlsMaterial,
    TlsRuntime,
    acme_router,
    build_quinn_config,
    build_rustls_config,
    generate_self_signed,
    install_crypto_provider,
    load_or_generate_material,
    new_runtime,
    parse_pem_material,
    serve_dual_tls,
    spawn_acme_http_listener,
};
pub use store::CertStore;
use thiserror::Error;

/// TLS certificate identity: DNS name or IP address (LE short-lived SAN).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsIdentity {
    Dns(String),
    Ip(IpAddr),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TlsError {
    #[error("TLS_HOSTNAME is empty")]
    EmptyHostname,
    #[error("TLS_HOSTNAME is invalid")]
    InvalidHostname,
    #[error("cannot generate self-signed cert without TLS_HOSTNAME")]
    NoIdentityForSelfSigned,
    #[error("failed to generate certificate")]
    CertGen,
    #[error("invalid PEM certificate or key")]
    InvalidPem,
    #[error("io error: {0}")]
    Io(String),
}

/// Parse `TLS_HOSTNAME` — dotted IP → [`TlsIdentity::Ip`], otherwise DNS.
pub fn parse_tls_identity(hostname: &str) -> Result<TlsIdentity, TlsError> {
    let trimmed = hostname.trim();
    if trimmed.is_empty() {
        return Err(TlsError::EmptyHostname);
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Ok(TlsIdentity::Ip(ip));
    }
    if trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Ok(TlsIdentity::Dns(trimmed.to_owned()));
    }
    Err(TlsError::InvalidHostname)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn parse_dns_hostname() {
        let id = parse_tls_identity("example.com").unwrap();
        assert_eq!(id, TlsIdentity::Dns("example.com".into()));
    }

    #[test]
    fn parse_ipv4_hostname() {
        let id = parse_tls_identity("192.168.1.1").unwrap();
        assert_eq!(id, TlsIdentity::Ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
    }

    #[test]
    fn parse_ipv6_hostname() {
        let id = parse_tls_identity("2001:db8::1").unwrap();
        assert_eq!(
            id,
            TlsIdentity::Ip(IpAddr::V6(Ipv6Addr::new(0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1)))
        );
    }

    #[test]
    fn parse_rejects_empty() {
        assert_eq!(parse_tls_identity(""), Err(TlsError::EmptyHostname));
        assert_eq!(parse_tls_identity("   "), Err(TlsError::EmptyHostname));
    }

    #[test]
    fn parse_rejects_invalid_dns() {
        assert_eq!(parse_tls_identity("bad host"), Err(TlsError::InvalidHostname));
    }
}
