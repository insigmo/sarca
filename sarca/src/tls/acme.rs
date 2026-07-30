use std::net::SocketAddr;

use super::TlsIdentity;

/// ACME certificate issuer (real http-01 client wired in Task 6).
pub trait AcmeIssuer: Send + Sync {
    fn directory_url(&self) -> &str;
    fn identity(&self) -> &TlsIdentity;
    fn http_addr(&self) -> SocketAddr;
}

/// Configuration for in-process ACME (order flow stubbed until Task 6).
#[derive(Debug, Clone)]
pub struct AcmeConfig {
    pub directory: String,
    pub http_addr: SocketAddr,
    pub identity: TlsIdentity,
}

impl AcmeConfig {
    pub fn new(directory: String, http_addr: SocketAddr, identity: TlsIdentity) -> Self {
        Self { directory, http_addr, identity }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    struct MockIssuer {
        config: AcmeConfig,
    }

    impl AcmeIssuer for MockIssuer {
        fn directory_url(&self) -> &str {
            &self.config.directory
        }

        fn identity(&self) -> &TlsIdentity {
            &self.config.identity
        }

        fn http_addr(&self) -> SocketAddr {
            self.config.http_addr
        }
    }

    #[test]
    fn mock_issuer_exposes_config() {
        let identity = TlsIdentity::Dns("example.com".into());
        let config = AcmeConfig::new(
            "https://acme-staging-v02.api.letsencrypt.org/directory".into(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8080),
            identity,
        );
        let issuer = MockIssuer { config };
        assert!(issuer.directory_url().contains("letsencrypt"));
        assert_eq!(issuer.http_addr().port(), 8080);
        assert!(matches!(issuer.identity(), TlsIdentity::Dns(h) if h == "example.com"));
    }
}
