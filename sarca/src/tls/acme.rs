use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

use chrono::{DateTime, Utc};
use instant_acme::{
    Account,
    AccountCredentials,
    ChallengeType,
    Identifier,
    NewAccount,
    NewOrder,
    OrderStatus,
    RetryPolicy,
};
use thiserror::Error;

use super::{CertStore, ChallengeStore, TlsIdentity};
use crate::config::Config;

/// Shared in-memory ACME http-01 challenge tokens.
pub type AcmeChallengeStore = ChallengeStore;

/// Let's Encrypt short-lived certificate profile (6-day lifetime).
pub const SHORTLIVED_PROFILE: &str = "shortlived";

const ACCOUNT_FILE: &str = "acme-account.json";

/// How long to wait for the CA to validate the http-01 challenge.
///
/// The library default gives up after 30s. Let's Encrypt regularly needs
/// longer than that (validation is queued and retried from several vantage
/// points), and the timeout surfaced as "timed out waiting for an order
/// update" while validation was still in flight.
const READY_POLICY: RetryPolicy = RetryPolicy::new()
    .initial_delay(Duration::from_secs(1))
    .backoff(1.4)
    .timeout(Duration::from_mins(3));

/// Issuance after validation is quicker, but still not always under 30s.
const CERTIFICATE_POLICY: RetryPolicy = RetryPolicy::new()
    .initial_delay(Duration::from_secs(1))
    .backoff(1.4)
    .timeout(Duration::from_secs(90));

/// Register an http-01 challenge response for `token`.
pub fn register_challenge(
    store: &AcmeChallengeStore,
    token: impl Into<String>,
    authorization: impl Into<String>,
) {
    store.write().expect("challenge lock").insert(token.into(), authorization.into());
}

/// ACME certificate issuer (real http-01 client wired when `TLS_HOSTNAME` is set).
pub trait AcmeIssuer: Send + Sync {
    fn directory_url(&self) -> &str;
    fn identity(&self) -> TlsIdentity;
    fn http_addr(&self) -> SocketAddr;
    fn challenges(&self) -> &AcmeChallengeStore;
}

/// Certificate identity shared with whoever may change it at runtime.
///
/// With no `TLS_HOSTNAME` the identity is the detected public IP, and that can
/// change under the running process (DHCP lease, NAT, failover). The watcher
/// writes the new address here and the next issuance picks it up.
pub type SharedIdentity = Arc<RwLock<TlsIdentity>>;

pub fn shared_identity(identity: TlsIdentity) -> SharedIdentity {
    Arc::new(RwLock::new(identity))
}

/// Configuration for in-process ACME certificate issuance.
#[derive(Debug, Clone)]
pub struct AcmeConfig {
    pub directory: String,
    pub http_addr: SocketAddr,
    pub identity: SharedIdentity,
    pub challenges: AcmeChallengeStore,
    pub account_path: PathBuf,
    /// Extra PEM root for the ACME client (private CA); `None` uses the system store.
    pub root_ca: Option<PathBuf>,
    /// Private key reused across issuances, so the pinned SPKI stays stable.
    pub key_path: PathBuf,
}

impl AcmeConfig {
    pub fn new(
        directory: String,
        http_addr: SocketAddr,
        identity: SharedIdentity,
        challenges: AcmeChallengeStore,
        account_path: PathBuf,
        root_ca: Option<PathBuf>,
        key_path: PathBuf,
    ) -> Self {
        Self {
            directory,
            http_addr,
            identity,
            challenges,
            account_path,
            root_ca,
            key_path,
        }
    }
}

/// Result of a successful ACME certificate issuance.
#[derive(Debug, Clone)]
pub struct IssuedCertificate {
    pub cert_pem: String,
    pub key_pem: String,
    pub not_after: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum AcmeError {
    #[error("ACME client error: {0}")]
    Client(#[from] instant_acme::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("no http-01 challenge offered for identifier")]
    NoHttp01Challenge,
    #[error("failed to build the certificate signing request")]
    CsrGen,
    #[error("invalid issued certificate")]
    InvalidCert,
    #[error("ACME directory URL is empty")]
    EmptyDirectory,
    #[error("ACME order never became ready (status: {0:?})")]
    OrderNotReady(OrderStatus),
}

/// Whether in-process ACME issuance is enabled (`SARCA_ACME=0` or empty directory disables it).
pub fn acme_enabled(config: &Config) -> bool {
    if config.acme_directory.trim().is_empty() {
        return false;
    }
    !matches!(
        std::env::var("SARCA_ACME").ok().as_deref(),
        Some("0" | "false" | "FALSE" | "no" | "NO")
    )
}

/// Convert [`TlsIdentity`] to an instant-acme [`Identifier`].
pub fn identity_to_identifier(identity: &TlsIdentity) -> Identifier {
    match identity {
        TlsIdentity::Dns(name) => Identifier::Dns(name.clone()),
        TlsIdentity::Ip(ip) => Identifier::Ip(*ip),
    }
}

/// Path for persisted ACME account credentials inside a cert store directory.
pub fn account_credentials_path(certs_dir: impl AsRef<Path>) -> PathBuf {
    certs_dir.as_ref().join(ACCOUNT_FILE)
}

/// In-process ACME issuer using instant-acme (http-01 via the shared challenge store).
#[derive(Debug, Clone)]
pub struct InstantAcmeIssuer {
    config: AcmeConfig,
}

impl InstantAcmeIssuer {
    pub fn new(config: AcmeConfig) -> Self {
        Self {
            config,
        }
    }

    pub fn from_parts(
        directory: String,
        http_addr: SocketAddr,
        identity: SharedIdentity,
        challenges: AcmeChallengeStore,
        cert_store: &CertStore,
        root_ca: Option<PathBuf>,
    ) -> Self {
        Self::new(AcmeConfig::new(
            directory,
            http_addr,
            identity,
            challenges,
            account_credentials_path(cert_store.dir()),
            root_ca,
            cert_store.key_path(),
        ))
    }

    /// Shared identity slot, so a public IP change can re-point issuance.
    pub fn identity_slot(&self) -> SharedIdentity {
        self.config.identity.clone()
    }

    /// Request and finalize a certificate from the configured ACME directory.
    pub async fn issue(&self) -> Result<IssuedCertificate, AcmeError> {
        issue_certificate(
            &self.config.directory,
            &self.identity(),
            &self.config.challenges,
            &self.config.account_path,
            self.config.root_ca.as_deref(),
            &self.config.key_path,
        )
        .await
    }
}

impl AcmeIssuer for InstantAcmeIssuer {
    fn directory_url(&self) -> &str {
        &self.config.directory
    }

    fn identity(&self) -> TlsIdentity {
        self.config.identity.read().expect("identity lock").clone()
    }

    fn http_addr(&self) -> SocketAddr {
        self.config.http_addr
    }

    fn challenges(&self) -> &AcmeChallengeStore {
        &self.config.challenges
    }
}

/// Stub issuer: exposes config and challenge store for tests.
#[derive(Debug, Clone)]
pub struct StubAcmeIssuer {
    config: AcmeConfig,
}

impl StubAcmeIssuer {
    pub fn new(config: AcmeConfig) -> Self {
        Self {
            config,
        }
    }
}

impl AcmeIssuer for StubAcmeIssuer {
    fn directory_url(&self) -> &str {
        &self.config.directory
    }

    fn identity(&self) -> TlsIdentity {
        self.config.identity.read().expect("identity lock").clone()
    }

    fn http_addr(&self) -> SocketAddr {
        self.config.http_addr
    }

    fn challenges(&self) -> &AcmeChallengeStore {
        &self.config.challenges
    }
}

/// Issue a certificate via ACME http-01 and return PEM material plus `notAfter`.
pub async fn issue_certificate(
    directory: &str,
    identity: &TlsIdentity,
    challenges: &AcmeChallengeStore,
    account_path: &Path,
    root_ca: Option<&Path>,
    key_path: &Path,
) -> Result<IssuedCertificate, AcmeError> {
    if directory.trim().is_empty() {
        return Err(AcmeError::EmptyDirectory);
    }

    // A private CA (step-ca, Pebble, the e2e mock) is not in the system trust
    // store, so its root has to be handed to the ACME client explicitly.
    let builder = match root_ca {
        Some(path) => Account::builder_with_root(path)?,
        None => Account::builder()?,
    };

    let account = load_or_create_account(builder, directory, account_path).await?;

    let identifiers = [identity_to_identifier(identity)];
    let mut order = new_order_with_profile(&account, &identifiers).await?;

    let mut authorizations = order.authorizations();
    while let Some(result) = authorizations.next().await {
        let mut authz = result?;
        let Some(mut challenge) = authz.challenge(ChallengeType::Http01) else {
            return Err(AcmeError::NoHttp01Challenge);
        };

        let token = challenge.token.clone();
        let key_auth = challenge.key_authorization();
        register_challenge(challenges, token, key_auth.as_str().to_owned());
        challenge.set_ready().await?;
    }

    let status = match order.poll_ready(&READY_POLICY).await {
        Ok(status) => status,
        Err(e) => {
            log_authorization_failures(&mut order).await;
            return Err(e.into());
        },
    };
    if status != OrderStatus::Ready {
        log_authorization_failures(&mut order).await;
        return Err(AcmeError::OrderNotReady(status));
    }

    // `Order::finalize` generates a throwaway keypair, so the public key would
    // change on every renewal (every ~6 days on the short-lived profile) and
    // break any client that pinned it. Sign our own CSR with the persisted key
    // instead.
    let key_pair = load_or_create_key(key_path).await?;
    let csr = build_csr(identity, &key_pair)?;
    order.finalize_csr(csr.der()).await?;
    let key_pem = key_pair.serialize_pem();
    let cert_pem = order.poll_certificate(&CERTIFICATE_POLICY).await?;

    challenges.write().expect("challenge lock").clear();

    let not_after = super::renew::parse_not_after(&cert_pem)?;

    Ok(IssuedCertificate {
        cert_pem,
        key_pem,
        not_after,
    })
}

/// Load the persisted private key, generating and saving one on first use.
///
/// A key that cannot be parsed is replaced rather than fatal: an unreadable
/// `key.pem` would otherwise block issuance forever.
async fn load_or_create_key(key_path: &Path) -> Result<rcgen::KeyPair, AcmeError> {
    if let Some(pem) = CertStore::load_pem_at(key_path).await? {
        match rcgen::KeyPair::from_pem(&pem) {
            Ok(key) => return Ok(key),
            Err(e) => {
                tracing::warn!(
                    "stored key at {} is unusable ({e}); generating a new one",
                    key_path.display()
                );
            },
        }
    }

    let key = rcgen::KeyPair::generate().map_err(|_| AcmeError::CsrGen)?;
    if let Some(parent) = key_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(key_path, key.serialize_pem()).await?;
    Ok(key)
}

/// CSR for the ACME identity, signed by the persisted key.
///
/// The SAN list holds the ACME identifier and nothing else: a CA rejects a CSR
/// naming identifiers the order did not authorize, so the extra `localhost` and
/// `127.0.0.1` names of the self-signed fallback must not appear here.
fn build_csr(
    identity: &TlsIdentity,
    key: &rcgen::KeyPair,
) -> Result<rcgen::CertificateSigningRequest, AcmeError> {
    let san = match identity {
        TlsIdentity::Dns(name) => name.clone(),
        TlsIdentity::Ip(ip) => ip.to_string(),
    };
    let mut params = rcgen::CertificateParams::new(vec![san]).map_err(|_| AcmeError::CsrGen)?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.serialize_request(key).map_err(|_| AcmeError::CsrGen)
}

/// Log why validation did not succeed.
///
/// `poll_ready` only reports that the order never became ready; the actionable
/// detail (unreachable http-01, wrong address, firewall) lives in the
/// per-authorization challenge error.
async fn log_authorization_failures(order: &mut instant_acme::Order) {
    let mut authorizations = order.authorizations();
    while let Some(result) = authorizations.next().await {
        let mut authz = match result {
            Ok(authz) => authz,
            Err(e) => {
                tracing::error!("ACME authorization could not be read: {e}");
                continue;
            },
        };
        let url = authz.url().to_owned();
        match authz.refresh().await {
            Ok(state) => {
                tracing::error!(
                    "ACME authorization {url}: status={:?}, challenges={:?}",
                    state.status,
                    state.challenges
                );
            },
            Err(e) => tracing::error!("ACME authorization {url} could not be refreshed: {e}"),
        }
    }
}

async fn new_order_with_profile(
    account: &Account,
    identifiers: &[Identifier],
) -> Result<instant_acme::Order, AcmeError> {
    let with_profile = NewOrder::new(identifiers).profile(SHORTLIVED_PROFILE);
    match account.new_order(&with_profile).await {
        Ok(order) => {
            tracing::info!("ACME order created with profile={SHORTLIVED_PROFILE}");
            Ok(order)
        },
        Err(e) => {
            tracing::warn!(
                "ACME shortlived profile unavailable ({e}); requesting default certificate lifetime"
            );
            Ok(account.new_order(&NewOrder::new(identifiers)).await?)
        },
    }
}

async fn load_or_create_account(
    builder: instant_acme::AccountBuilder,
    directory: &str,
    account_path: &Path,
) -> Result<Account, AcmeError> {
    if account_path.is_file() {
        let json = tokio::fs::read_to_string(account_path).await?;
        let credentials: AccountCredentials = serde_json::from_str(&json)?;
        return builder.from_credentials(credentials).await.map_err(AcmeError::from);
    }

    let (account, credentials) = builder
        .create(
            &NewAccount {
                contact: &[],
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            directory.to_owned(),
            None,
        )
        .await?;

    if let Some(parent) = account_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string_pretty(&credentials)?;
    tokio::fs::write(account_path, json).await?;
    tracing::info!("ACME account created and saved to {}", account_path.display());

    Ok(account)
}

/// Persist issued PEM material to a cert store.
pub async fn save_issued(store: &CertStore, issued: &IssuedCertificate) -> Result<(), AcmeError> {
    store.save_cert(&issued.cert_pem).await?;
    store.save_key(&issued.key_pem).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    fn test_config() -> AcmeConfig {
        let identity = shared_identity(TlsIdentity::Dns("example.com".into()));
        let challenges =
            std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));
        AcmeConfig::new(
            "https://acme-staging-v02.api.letsencrypt.org/directory".into(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8080),
            identity,
            challenges,
            PathBuf::from("/tmp/acme-account.json"),
            None,
            PathBuf::from("/tmp/acme-key.pem"),
        )
    }

    #[test]
    fn stub_issuer_exposes_config() {
        let config = test_config();
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

    #[tokio::test]
    async fn key_is_generated_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.pem");

        let first = load_or_create_key(&path).await.unwrap();
        assert!(path.exists());
        let second = load_or_create_key(&path).await.unwrap();

        // The public key is what clients pin, so it is what must survive a
        // renewal, not just the file.
        assert_eq!(first.public_key_der(), second.public_key_der());
    }

    #[tokio::test]
    async fn unusable_key_is_replaced_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.pem");
        tokio::fs::write(&path, "not a key").await.unwrap();

        let key = load_or_create_key(&path).await.unwrap();
        let reloaded = load_or_create_key(&path).await.unwrap();
        assert_eq!(key.public_key_der(), reloaded.public_key_der());
    }

    #[test]
    fn csr_names_only_the_acme_identity() {
        let key = rcgen::KeyPair::generate().unwrap();
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10));
        let csr = build_csr(&TlsIdentity::Ip(ip), &key).unwrap();

        use x509_parser::prelude::FromDer;
        let (_, parsed) =
            x509_parser::certification_request::X509CertificationRequest::from_der(csr.der())
                .unwrap();
        let sans: Vec<_> = parsed
            .requested_extensions()
            .unwrap()
            .filter_map(|ext| {
                match ext {
                    x509_parser::extensions::ParsedExtension::SubjectAlternativeName(san) => {
                        Some(san.general_names.clone())
                    },
                    _ => None,
                }
            })
            .flatten()
            .collect();
        assert_eq!(sans, vec![x509_parser::extensions::GeneralName::IPAddress(&[203, 0, 113, 10])]);
    }

    #[test]
    fn identity_to_identifier_dns_and_ip() {
        assert_eq!(
            identity_to_identifier(&TlsIdentity::Dns("example.com".into())),
            Identifier::Dns("example.com".into())
        );
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10));
        assert_eq!(identity_to_identifier(&TlsIdentity::Ip(ip)), Identifier::Ip(ip));
    }

    #[test]
    fn acme_enabled_respects_env_and_directory() {
        use std::sync::Mutex;

        static ENV_LOCK: Mutex<()> = Mutex::new(());

        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("PORT", "8001");
        std::env::set_var("WORKERS", "2");
        std::env::set_var("CHANNEL_CAPACITY", "8");
        std::env::set_var("SUPERUSER_EMAIL", "a@b.c");
        std::env::set_var("SUPERUSER_PASS", "pass");
        std::env::set_var("ACCESS_TOKEN_EXPIRE_IN_SECS", "1800");
        std::env::set_var("REFRESH_TOKEN_EXPIRE_IN_DAYS", "14");
        std::env::set_var("SECRET_KEY", "secret");

        let mut cfg = crate::config::Config::new().expect("config");
        cfg.acme_directory = "https://acme-v02.api.letsencrypt.org/directory".into();
        assert!(acme_enabled(&cfg));

        std::env::set_var("SARCA_ACME", "0");
        assert!(!acme_enabled(&cfg));
        std::env::remove_var("SARCA_ACME");

        cfg.acme_directory.clear();
        assert!(!acme_enabled(&cfg));
    }

    #[test]
    fn instant_issuer_implements_trait() {
        let issuer = InstantAcmeIssuer::new(test_config());
        assert!(issuer.directory_url().contains("letsencrypt"));
        assert_eq!(issuer.http_addr().port(), 8080);
    }
}
