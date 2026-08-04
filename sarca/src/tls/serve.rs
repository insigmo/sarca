use std::{
    io::Cursor,
    net::SocketAddr,
    sync::{
        Arc,
        RwLock,
        atomic::{AtomicU32, Ordering},
    },
};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use h3_quinn::quinn;
use hyper_util::service::TowerToHyperService;
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer},
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
};
use tokio::sync::Notify;
use tokio_rustls::TlsAcceptor;

use super::{CertStore, TlsError, TlsIdentity};

/// In-memory ACME http-01 challenge tokens (`token` → key authorization).
pub type ChallengeStore = Arc<RwLock<std::collections::HashMap<String, String>>>;

/// Loaded or generated TLS key material.
pub struct TlsMaterial {
    pub cert_chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

/// Certificate holder shared by the TCP and QUIC rustls configs.
///
/// Both listeners resolve their certificate through this one object, so a
/// renewed certificate reaches HTTP/3 without a process restart: QUIC's
/// `ServerConfig` is built once, but it asks the resolver on every handshake.
#[derive(Debug)]
pub struct CertResolver {
    current: RwLock<Arc<CertifiedKey>>,
}

impl CertResolver {
    pub fn new(material: &TlsMaterial) -> Result<Arc<Self>, TlsError> {
        Ok(Arc::new(Self {
            current: RwLock::new(certified_key(material)?),
        }))
    }

    /// Swap in freshly issued material. Live connections keep their old
    /// certificate; every new handshake gets the new one.
    pub fn set(&self, material: &TlsMaterial) -> Result<(), TlsError> {
        let key = certified_key(material)?;
        *self.current.write().expect("cert resolver lock") = key;
        Ok(())
    }
}

impl ResolvesServerCert for CertResolver {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.current.read().expect("cert resolver lock").clone())
    }
}

fn certified_key(material: &TlsMaterial) -> Result<Arc<CertifiedKey>, TlsError> {
    let signing = rustls::crypto::ring::sign::any_supported_type(&material.key)
        .map_err(|_| TlsError::InvalidPem)?;
    Ok(Arc::new(CertifiedKey::new(material.cert_chain.clone(), signing)))
}

/// Runtime TLS settings shared by TCP HTTPS and QUIC listeners.
#[derive(Clone)]
pub struct TlsRuntime {
    pub https_addr: SocketAddr,
    pub acme_addr: SocketAddr,
    pub challenges: ChallengeStore,
    pub https_redirect_base: String,
    resolver: Arc<CertResolver>,
    server_config: Arc<RwLock<Arc<ServerConfig>>>,
    quinn_config: quinn::ServerConfig,
    renew_signal: Arc<Notify>,
    h3_failures: Arc<AtomicU32>,
}

const QUIC_ALPN: &[&[u8]] = &[b"h3"];
const TCP_ALPN: &[&[u8]] = &[b"h2", b"http/1.1"];

/// Consecutive HTTP/3 handshake failures that trigger a certificate refresh.
///
/// A broken or expired certificate makes every QUIC handshake fail while TCP
/// TLS may still limp along on a cached session, so repeated H3 failures are
/// the earliest signal that the cert needs re-issuing.
const H3_FAILURE_THRESHOLD: u32 = 3;

/// Load PEM from [`CertStore`] or generate a self-signed cert for first boot.
pub async fn load_or_generate_material(
    store: &CertStore,
    identity: Option<&TlsIdentity>,
) -> Result<TlsMaterial, TlsError> {
    if let (Some(cert_pem), Some(key_pem)) = (
        store.load_cert().await.map_err(|e| TlsError::Io(e.to_string()))?,
        store.load_key().await.map_err(|e| TlsError::Io(e.to_string()))?,
    ) {
        return parse_pem_material(&cert_pem, &key_pem);
    }

    let id = identity.ok_or(TlsError::NoIdentityForSelfSigned)?;
    let (cert_pem, key_pem, material) = generate_self_signed_pem(id)?;
    store.save_cert(&cert_pem).await.map_err(|e| TlsError::Io(e.to_string()))?;
    store.save_key(&key_pem).await.map_err(|e| TlsError::Io(e.to_string()))?;
    Ok(material)
}

pub fn generate_self_signed(identity: &TlsIdentity) -> Result<TlsMaterial, TlsError> {
    generate_self_signed_pem(identity).map(|(_, _, material)| material)
}

fn generate_self_signed_pem(
    identity: &TlsIdentity,
) -> Result<(String, String, TlsMaterial), TlsError> {
    let mut subject_alt_names = match identity {
        TlsIdentity::Dns(name) => vec![name.clone(), "localhost".to_owned()],
        TlsIdentity::Ip(ip) => vec![ip.to_string(), "127.0.0.1".to_owned(), "localhost".to_owned()],
    };
    subject_alt_names.sort_unstable();
    subject_alt_names.dedup();

    let cert =
        rcgen::generate_simple_self_signed(subject_alt_names).map_err(|_| TlsError::CertGen)?;
    let cert_pem = cert.cert.pem();
    let key_pem = cert.key_pair.serialize_pem();
    let key = PrivateKeyDer::Pkcs8(cert.key_pair.serialize_der().into());
    let cert_chain = vec![CertificateDer::from(cert.cert)];
    Ok((
        cert_pem,
        key_pem,
        TlsMaterial {
            cert_chain,
            key,
        },
    ))
}

pub fn parse_pem_material(cert_pem: &str, key_pem: &str) -> Result<TlsMaterial, TlsError> {
    let mut cert_reader = Cursor::new(cert_pem.as_bytes());
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| TlsError::InvalidPem)?;

    let mut key_reader = Cursor::new(key_pem.as_bytes());
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|_| TlsError::InvalidPem)?
        .ok_or(TlsError::InvalidPem)?;

    if certs.is_empty() {
        return Err(TlsError::InvalidPem);
    }

    Ok(TlsMaterial {
        cert_chain: certs,
        key,
    })
}

pub fn build_rustls_config(resolver: Arc<CertResolver>, alpn: &[&[u8]]) -> Arc<ServerConfig> {
    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .expect("protocols")
            .with_no_client_auth()
            .with_cert_resolver(resolver);
    config.alpn_protocols = alpn.iter().map(|p| (*p).to_vec()).collect();
    config.max_early_data_size = u32::MAX;
    Arc::new(config)
}

pub fn build_quinn_config(rustls: Arc<ServerConfig>) -> quinn::ServerConfig {
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(rustls).expect("quic tls"),
    ));
    server_config.migration(true);
    let transport = Arc::get_mut(&mut server_config.transport).expect("transport config");
    transport.max_concurrent_bidi_streams(256u32.into()).max_concurrent_uni_streams(256u32.into());
    server_config
}

pub fn new_runtime(
    https_addr: SocketAddr,
    acme_addr: SocketAddr,
    material: &TlsMaterial,
    https_redirect_base: String,
    challenges: ChallengeStore,
) -> TlsRuntime {
    let resolver = CertResolver::new(material).expect("valid tls material");
    let tcp = build_rustls_config(resolver.clone(), TCP_ALPN);
    let quic = build_rustls_config(resolver.clone(), QUIC_ALPN);
    TlsRuntime {
        https_addr,
        acme_addr,
        challenges,
        https_redirect_base,
        resolver,
        server_config: Arc::new(RwLock::new(tcp)),
        quinn_config: build_quinn_config(quic),
        renew_signal: Arc::new(Notify::new()),
        h3_failures: Arc::new(AtomicU32::new(0)),
    }
}

impl TlsRuntime {
    /// Hot-reload the certificate for both TCP TLS and HTTP/3.
    pub fn reload_material(&self, material: &TlsMaterial) -> Result<(), TlsError> {
        self.resolver.set(material)?;
        self.h3_failures.store(0, Ordering::Relaxed);
        tracing::info!("TLS certificate hot-reloaded (TCP + HTTP/3)");
        Ok(())
    }

    /// Ask the renewal task to re-issue as soon as it can.
    pub fn request_renewal(&self, reason: &str) {
        tracing::warn!("requesting certificate renewal: {reason}");
        self.renew_signal.notify_one();
    }

    /// Waker the renewal task parks on between scheduled renewals.
    pub fn renew_signal(&self) -> Arc<Notify> {
        self.renew_signal.clone()
    }

    /// A QUIC handshake failed. Enough of them in a row means the certificate
    /// is the likely cause, so kick off a renewal to get HTTP/3 back.
    fn note_h3_failure(&self) {
        let failures = self.h3_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if failures >= H3_FAILURE_THRESHOLD {
            self.h3_failures.store(0, Ordering::Relaxed);
            self.request_renewal("HTTP/3 handshakes failing repeatedly");
        }
    }

    fn note_h3_success(&self) {
        self.h3_failures.store(0, Ordering::Relaxed);
    }
}

/// Start the ACME http-01 + redirect listener (shared challenge store).
pub fn spawn_acme_http_listener(
    acme_addr: SocketAddr,
    challenges: ChallengeStore,
    https_redirect_base: String,
) -> tokio::task::JoinHandle<()> {
    let router = acme_router(challenges, https_redirect_base);
    tokio::spawn(async move {
        if let Err(e) = serve_acme_http(acme_addr, router).await {
            tracing::error!("ACME HTTP listener failed: {e}");
        }
    })
}

pub fn acme_router(challenges: ChallengeStore, https_redirect_base: String) -> Router {
    Router::new()
        .route(
            "/.well-known/acme-challenge/{token}",
            get(move |axum::extract::Path(token): axum::extract::Path<String>| {
                let challenges = challenges.clone();
                async move {
                    let map = challenges.read().expect("challenge lock");
                    map.get(&token).map_or_else(
                        || StatusCode::NOT_FOUND.into_response(),
                        |body| {
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "text/plain")
                                .body(Body::from(body.clone()))
                                .unwrap()
                        },
                    )
                }
            }),
        )
        .fallback(move |req: Request<Body>| {
            let base = https_redirect_base.clone();
            async move {
                let path_and_query =
                    req.uri().path_and_query().map_or("/", axum::http::uri::PathAndQuery::as_str);
                let target = format!("{base}{path_and_query}");
                Response::builder()
                    .status(StatusCode::MOVED_PERMANENTLY)
                    .header(header::LOCATION, target)
                    .body(Body::empty())
                    .unwrap()
                    .into_response()
            }
        })
}

/// Install rustls crypto provider (required for rustls 0.23).
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Serve Axum over TCP TLS, QUIC HTTP/3, and ACME/redirect port concurrently.
pub async fn serve_dual_tls(
    router: Router,
    ui_dir: std::path::PathBuf,
    runtime: TlsRuntime,
    acme_task: Option<tokio::task::JoinHandle<()>>,
) {
    let tcp_router = router.clone();
    let h3_router = router;
    let acme_router = acme_router(runtime.challenges.clone(), runtime.https_redirect_base.clone());

    let https_addr = runtime.https_addr;
    let acme_addr = runtime.acme_addr;
    let tcp_cfg = runtime.server_config.clone();
    let h3_runtime = runtime.clone();

    eprintln!();
    eprintln!("========================================");
    eprintln!("  Sarca is running (TLS)");
    eprintln!("  HTTPS:   {}", runtime.https_redirect_base);
    eprintln!("  HTTP/3:  {} (UDP)", runtime.https_redirect_base);
    eprintln!("  ACME:    http://127.0.0.1:{}", acme_addr.port());
    eprintln!("  UI dir:  {}", ui_dir.display());
    eprintln!("========================================");
    eprintln!();

    let tcp_task = tokio::spawn(async move {
        if let Err(e) = serve_tcp_tls(https_addr, tcp_router, tcp_cfg).await {
            tracing::error!("TCP TLS listener failed: {e}");
        }
    });

    let h3_task = tokio::spawn(async move {
        if let Err(e) = serve_h3(https_addr, h3_router, h3_runtime).await {
            tracing::error!("HTTP/3 listener failed: {e}");
        }
    });

    let acme_task = acme_task.unwrap_or_else(|| {
        tokio::spawn(async move {
            if let Err(e) = serve_acme_http(acme_addr, acme_router).await {
                tracing::error!("ACME HTTP listener failed: {e}");
            }
        })
    });

    let _ = tokio::join!(tcp_task, h3_task, acme_task);
}

async fn serve_tcp_tls(
    addr: SocketAddr,
    router: Router,
    cfg: Arc<RwLock<Arc<ServerConfig>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("TCP TLS listening on {addr}");

    loop {
        let (stream, remote) = listener.accept().await?;
        let tls_cfg = cfg.read().expect("tls lock").clone();
        let acceptor = TlsAcceptor::from(tls_cfg);
        let router = router.clone();
        tokio::spawn(async move {
            match acceptor.accept(stream).await {
                Ok(tls_stream) => {
                    let io = hyper_util::rt::TokioIo::new(tls_stream);
                    let service = TowerToHyperService::new(router);
                    if let Err(e) = hyper_util::server::conn::auto::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    )
                    .serve_connection(io, service)
                    .await
                    {
                        tracing::debug!("TCP TLS connection from {remote} ended: {e}");
                    }
                },
                Err(e) => tracing::debug!("TLS handshake from {remote} failed: {e}"),
            }
        });
    }
}

async fn serve_h3(
    addr: SocketAddr,
    router: Router,
    runtime: TlsRuntime,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = quinn::Endpoint::server(runtime.quinn_config.clone(), addr)?;
    tracing::info!("HTTP/3 listening on {addr} (UDP)");

    while let Some(incoming) = endpoint.accept().await {
        let app = router.clone();
        let runtime = runtime.clone();
        tokio::spawn(async move {
            match handle_h3_connection(incoming, app, &runtime).await {
                Ok(()) => runtime.note_h3_success(),
                Err(e) => {
                    tracing::debug!("HTTP/3 connection error: {e}");
                    runtime.note_h3_failure();
                },
            }
        });
    }
    Ok(())
}

async fn handle_h3_connection(
    incoming: quinn::Incoming,
    app: Router,
    runtime: &TlsRuntime,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn = incoming.await?;
    // Handshake completed, so the certificate is fine no matter what the
    // request stream does afterwards.
    runtime.note_h3_success();
    let mut h3_conn = h3::server::builder().build(h3_quinn::Connection::new(conn)).await?;
    loop {
        match h3_conn.accept().await {
            Ok(Some(resolver)) => {
                let app = app.clone();
                tokio::spawn(async move {
                    if let Err(e) = h3_axum::serve_h3_with_axum(app, resolver).await {
                        tracing::debug!("HTTP/3 request error: {e}");
                    }
                });
            },
            Ok(None) => break,
            Err(e) => {
                if h3_axum::is_graceful_h3_close(&e) {
                    break;
                }
                return Err(e.into());
            },
        }
    }
    Ok(())
}

async fn serve_acme_http(
    addr: SocketAddr,
    router: Router,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("ACME HTTP listening on {addr}");
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_signed_roundtrip_parse() {
        let id = TlsIdentity::Dns("test.local".into());
        let (cert_pem, key_pem, _) = generate_self_signed_pem(&id).unwrap();
        let parsed = parse_pem_material(&cert_pem, &key_pem).unwrap();
        assert!(!parsed.cert_chain.is_empty());
    }

    #[test]
    fn resolver_swap_is_visible_to_both_listeners() {
        let first = generate_self_signed(&TlsIdentity::Dns("first.local".into())).unwrap();
        let second = generate_self_signed(&TlsIdentity::Dns("second.local".into())).unwrap();

        let resolver = CertResolver::new(&first).unwrap();
        // The QUIC config is built once and never rebuilt, so the shared
        // resolver is the only path a renewed cert has into HTTP/3.
        let _tcp = build_rustls_config(resolver.clone(), TCP_ALPN);
        let _quic = build_quinn_config(build_rustls_config(resolver.clone(), QUIC_ALPN));

        let before = resolver.current.read().unwrap().cert[0].clone();
        resolver.set(&second).unwrap();
        let after = resolver.current.read().unwrap().cert[0].clone();

        assert_eq!(before, first.cert_chain[0]);
        assert_eq!(after, second.cert_chain[0]);
    }
}
