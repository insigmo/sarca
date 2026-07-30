mod acme;
mod renew;
mod store;

pub use acme::{AcmeConfig, AcmeIssuer};
pub use renew::renew_at;
pub use store::CertStore;

use std::net::IpAddr;

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
        assert_eq!(id, TlsIdentity::Ip(IpAddr::V6(Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 1))));
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
