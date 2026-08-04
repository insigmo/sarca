mod acme;
mod public_ip;
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
    SharedIdentity,
    StubAcmeIssuer,
    acme_enabled,
    identity_to_identifier,
    register_challenge,
    save_issued,
    shared_identity,
};
pub use public_ip::{detect_public_ip, spawn_public_ip_watch};
pub use renew::{parse_not_after, renew_at, spawn_renewal_task};
pub use serve::{
    CertResolver,
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

/// Base URL clients should use for this identity.
///
/// IPv6 needs brackets, and the port is omitted when it is the HTTPS default so
/// the printed URL matches what a browser shows.
pub fn identity_base_url(identity: &TlsIdentity, port: u16) -> String {
    let host = match identity {
        TlsIdentity::Dns(name) => name.clone(),
        TlsIdentity::Ip(IpAddr::V4(ip)) => ip.to_string(),
        TlsIdentity::Ip(IpAddr::V6(ip)) => format!("[{ip}]"),
    };
    if port == 443 { format!("https://{host}") } else { format!("https://{host}:{port}") }
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
    fn base_url_omits_default_port_and_brackets_ipv6() {
        let dns = TlsIdentity::Dns("sarca.example".into());
        assert_eq!(identity_base_url(&dns, 443), "https://sarca.example");
        assert_eq!(identity_base_url(&dns, 8443), "https://sarca.example:8443");

        let v4 = TlsIdentity::Ip(IpAddr::V4(Ipv4Addr::new(176, 85, 145, 128)));
        assert_eq!(identity_base_url(&v4, 443), "https://176.85.145.128");

        let v6 = TlsIdentity::Ip(IpAddr::V6(Ipv6Addr::new(0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1)));
        assert_eq!(identity_base_url(&v6, 443), "https://[2001:db8::1]");
        assert_eq!(identity_base_url(&v6, 8443), "https://[2001:db8::1]:8443");
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
