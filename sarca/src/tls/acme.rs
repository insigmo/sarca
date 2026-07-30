use std::net::SocketAddr;

use super::ChallengeStore;
use super::TlsIdentity;

/// Shared in-memory ACME http-01 challenge tokens.
pub type AcmeChallengeStore = ChallengeStore;

/// Register an http-01 challenge response for `token`.
pub fn register_challenge(store: &AcmeChallengeStore, token: impl Into<String>, authorization: impl Into<String>) {
    store
        .write()
        .expect("challenge lock")
        .insert(token.into(), authorization.into());
}

/// ACME certificate issuer (real http-01 client wired when `TLS_HOSTNAME` is set).
pub trait AcmeIssuer: Send + Sync {
    fn directory_url(&self) -> &str;
    fn identity(&self) -> &TlsIdentity;
    fn http_addr(&self) -> SocketAddr;
    fn challenges(&self) -> &AcmeChallengeStore;
}

/// Configuration for in-process ACME (order flow stubbed until full instant-acme wiring).
#[derive(Debug, Clone)]
pub struct AcmeConfig {
    pub directory: String,
    pub http_addr: SocketAddr,
    pub identity: TlsIdentity,
    pub challenges: AcmeChallengeStore,
}

impl AcmeConfig {
    pub fn new(
        directory: String,
        http_addr: SocketAddr,
        identity: TlsIdentity,
        challenges: AcmeChallengeStore,
    ) -> Self {
        Self { directory, http_addr, identity, challenges }
    }
}

/// Stub issuer: exposes config and challenge store for Task 6 wiring.
#[derive(Debug, Clone)]
pub struct StubAcmeIssuer {
    config: AcmeConfig,
}

impl StubAcmeIssuer {
    pub fn new(config: AcmeConfig) -> Self {
        Self { config }
    }
}

impl AcmeIssuer for StubAcmeIssuer {
    fn directory_url(&self) -> &str {
        &self.config.directory
    }

    fn identity(&self) -> &TlsIdentity {
        &self.config.identity
    }

    fn http_addr(&self) -> SocketAddr {
        self.config.http_addr
    }

    fn challenges(&self) -> &AcmeChallengeStore {
        &self.config.challenges
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::{Arc, RwLock};

    use super::*;

    #[test]
    fn stub_issuer_exposes_config() {
        let identity = TlsIdentity::Dns("example.com".into());
        let challenges = Arc::new(RwLock::new(std::collections::HashMap::new()));
        let config = AcmeConfig::new(
            "https://acme-staging-v02.api.letsencrypt.org/directory".into(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8080),
            identity,
            challenges.clone(),
        );
        let issuer = StubAcmeIssuer::new(config);
        assert!(issuer.directory_url().contains("letsencrypt"));
        assert_eq!(issuer.http_addr().port(), 8080);
        assert!(matches!(issuer.identity(), TlsIdentity::Dns(h) if h == "example.com"));
        register_challenge(issuer.challenges(), "tok", "auth");
        assert_eq!(
            issuer.challenges().read().unwrap().get("tok").map(String::as_str),
            Some("auth")
        );
    }
}
