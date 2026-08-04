//! Trust-on-first-use certificate pinning.
//!
//! A Sarca server gets its certificate from Let's Encrypt for a single public
//! address, so reaching the same server over a LAN address or over loopback
//! always fails hostname validation. Instead of teaching every client a CA, we
//! remember the server's public key the first time we see it and accept that
//! key afterwards, on any address.
//!
//! What is pinned is `SHA-256(subjectPublicKeyInfo)`, not the certificate:
//! short-lived certificates are reissued every few days, and the server keeps
//! its private key across renewals (see `sarca::tls::acme`), so the SPKI hash
//! survives a renewal while a leaf fingerprint would not.

use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use sha2::{Digest, Sha256};

/// Persistent map of host to pinned SPKI hash.
pub trait PinStore: Send + Sync {
    fn get(&self, host: &str) -> Option<[u8; 32]>;
    fn put(&self, host: &str, pin: [u8; 32]);
}

static PIN_STORE: OnceLock<Arc<dyn PinStore>> = OnceLock::new();

/// Install the process-wide pin store.
///
/// Called once during client startup, before any HTTP client is built. Later
/// calls are ignored, and return `false` so a caller can notice the misuse.
pub fn set_pin_store(store: Arc<dyn PinStore>) -> bool {
    PIN_STORE.set(store).is_ok()
}

fn pin_store() -> Option<&'static Arc<dyn PinStore>> {
    PIN_STORE.get()
}

/// TLS config with TOFU pinning, or `None` when no pin store was installed.
///
/// Without a store the caller keeps reqwest's default configuration, so a
/// build that never calls [`set_pin_store`] behaves exactly as before.
pub fn pinned_tls_config() -> Option<ClientConfig> {
    pin_store().map(|store| build_config(store.clone()))
}

fn build_config(store: Arc<dyn PinStore>) -> ClientConfig {
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let provider = CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));

    let mut config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("default protocol versions are supported")
        .with_root_certificates(roots.clone())
        .with_no_client_auth();

    config
        .dangerous()
        .set_certificate_verifier(Arc::new(PinnedVerifier::new(store, roots, provider)));
    config
}

/// SHA-256 over the certificate's `subjectPublicKeyInfo`.
pub fn spki_pin(cert: &CertificateDer<'_>) -> Result<[u8; 32], rustls::Error> {
    use x509_parser::prelude::FromDer;

    let (_, parsed) = x509_parser::certificate::X509Certificate::from_der(cert.as_ref())
        .map_err(|_| rustls::Error::General("unparseable server certificate".into()))?;
    let spki = parsed.tbs_certificate.subject_pki.raw;
    Ok(Sha256::digest(spki).into())
}

/// Verifier that falls back to a stored public-key pin.
///
/// A normally valid chain is accepted without touching the store, so a public
/// address keeps its full web PKI guarantees. Only when the chain does not
/// validate (typically a name mismatch on a LAN address) does the pin decide:
/// an unknown host is remembered and accepted, a host whose key changed is
/// rejected.
pub struct PinnedVerifier {
    store: Arc<dyn PinStore>,
    inner: Arc<rustls::client::WebPkiServerVerifier>,
}

impl PinnedVerifier {
    pub fn new(
        store: Arc<dyn PinStore>,
        roots: RootCertStore,
        provider: Arc<CryptoProvider>,
    ) -> Self {
        let inner =
            rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
                .build()
                .expect("web PKI verifier with built-in roots");
        Self { store, inner }
    }
}

impl fmt::Debug for PinnedVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PinnedVerifier")
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let webpki_err = match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => return Ok(verified),
            Err(e) => e,
        };

        let host = server_name.to_str().into_owned();
        let pin = spki_pin(end_entity)?;
        match self.store.get(&host) {
            Some(known) if known == pin => Ok(ServerCertVerified::assertion()),
            Some(_) => Err(rustls::Error::General(format!(
                "server key for {host} changed since it was first trusted"
            ))),
            None => {
                // First sight of this host: remember the key. The chain itself
                // is untrusted, which is exactly what a pin is for.
                let _ = webpki_err;
                self.store.put(&host, pin);
                Ok(ServerCertVerified::assertion())
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Mutex, RwLock},
    };

    use rustls::pki_types::CertificateDer;

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        pins: RwLock<HashMap<String, [u8; 32]>>,
    }

    impl PinStore for MemoryStore {
        fn get(&self, host: &str) -> Option<[u8; 32]> {
            self.pins.read().unwrap().get(host).copied()
        }

        fn put(&self, host: &str, pin: [u8; 32]) {
            self.pins.write().unwrap().insert(host.to_owned(), pin);
        }
    }

    /// Serializes the process-wide default provider install.
    static PROVIDER: Mutex<()> = Mutex::new(());

    fn verifier(store: Arc<dyn PinStore>) -> PinnedVerifier {
        let _guard = PROVIDER.lock().unwrap();
        let provider = CryptoProvider::get_default().cloned().unwrap_or_else(|| {
            let p = Arc::new(rustls::crypto::ring::default_provider());
            let _ = CryptoProvider::install_default((*p).clone());
            p
        });
        let roots = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        PinnedVerifier::new(store, roots, provider)
    }

    /// Certificate for `san`, signed by its own throwaway key.
    fn self_signed(san: &str, key: &rcgen::KeyPair) -> CertificateDer<'static> {
        let params = rcgen::CertificateParams::new(vec![san.to_owned()]).unwrap();
        let cert = params.self_signed(key).unwrap();
        cert.der().clone()
    }

    fn verify(
        v: &PinnedVerifier,
        cert: &CertificateDer<'_>,
        host: &str,
    ) -> Result<(), rustls::Error> {
        v.verify_server_cert(
            cert,
            &[],
            &ServerName::try_from(host.to_owned()).unwrap(),
            &[],
            UnixTime::now(),
        )
        .map(|_| ())
    }

    #[test]
    fn first_sight_is_remembered_then_accepted() {
        let store = Arc::new(MemoryStore::default());
        let v = verifier(store.clone());
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = self_signed("203.0.113.10", &key);

        // Untrusted chain, unknown host: accepted and recorded.
        verify(&v, &cert, "192.168.1.39").unwrap();
        assert_eq!(store.get("192.168.1.39"), Some(spki_pin(&cert).unwrap()));

        // Same key, new certificate: a renewal must not break the pin.
        let renewed = self_signed("203.0.113.10", &key);
        assert_ne!(cert, renewed);
        verify(&v, &renewed, "192.168.1.39").unwrap();
    }

    #[test]
    fn changed_key_is_rejected() {
        let store = Arc::new(MemoryStore::default());
        let v = verifier(store.clone());
        let cert = self_signed("203.0.113.10", &rcgen::KeyPair::generate().unwrap());
        verify(&v, &cert, "192.168.1.39").unwrap();

        let impostor = self_signed("203.0.113.10", &rcgen::KeyPair::generate().unwrap());
        let err = verify(&v, &impostor, "192.168.1.39").unwrap_err();
        assert!(err.to_string().contains("changed"), "{err}");
        // The stored pin stays put, so the rejection is not self-healing.
        assert_eq!(store.get("192.168.1.39"), Some(spki_pin(&cert).unwrap()));
    }

    #[test]
    fn pin_is_the_key_not_the_certificate() {
        let key = rcgen::KeyPair::generate().unwrap();
        let a = self_signed("a.example", &key);
        let b = self_signed("b.example", &key);
        assert_eq!(spki_pin(&a).unwrap(), spki_pin(&b).unwrap());

        let other = self_signed("a.example", &rcgen::KeyPair::generate().unwrap());
        assert_ne!(spki_pin(&a).unwrap(), spki_pin(&other).unwrap());
    }
}
